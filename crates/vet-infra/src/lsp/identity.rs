//! The identity of a language server (I2): a hash of everything its answers
//! depend on apart from the tree it runs on. Two servers with equal
//! fingerprints answer equal questions equally, so an answer is reused only
//! under the fingerprint it was given under.
//!
//! What goes in: the program (path and content), its `--version`, its
//! arguments, the options and client capabilities it is initialized with, the
//! compiler it is told about, the toolchain it finds in the root, and the
//! configuration it reads outside the repository (`UserConfig` of each
//! language). Hashing a binary and probing its version are memoized by path,
//! size and modification time, so a start or an `identify` does not read a
//! 50 MB executable again unless it was replaced.

use std::collections::HashMap;
use std::convert::Infallible;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use serde_json::Value;
use vet_app::ports::ServerCommand;
use vet_core::content::Hash;
use vet_engine::{Computed, Engine, Kind, Limits, Retention};
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{Backend, BackendId, LaunchError, ServerIdentity};

use super::client_capabilities;
use super::setup;
use crate::budget::{Budget, Budgets, PROBES, SEMANTIC_IO};
use crate::exec::{Host, is_file};
use crate::language::{self, Install, Language, SchemaClient, UserFile};
use crate::restore::record::Digest;
use crate::schemas::{self, SchemaSource};
use crate::servers::{capture, probe_version};

/// Bumped when what the fingerprint is made of changes, so no old fingerprint equals a new one.
const FORMAT: u32 = 2;

/// The user's home and environment, as the servers see them.
#[derive(Clone, Debug, Default)]
pub struct UserEnv {
    pub home: Option<PathBuf>,
    pub vars: HashMap<String, String>,
}

impl UserEnv {
    pub fn current() -> Self {
        Self {
            home: dirs::home_dir(),
            vars: std::env::vars_os()
                .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
                .collect(),
        }
    }

    fn file(&self, file: &UserFile) -> Option<PathBuf> {
        match file {
            UserFile::Home(path) => Some(self.home.as_ref()?.join(path)),
            UserFile::Under { var, fallback, file } => {
                let dir = match self.vars.get(*var) {
                    Some(dir) => PathBuf::from(dir),
                    None => self.home.as_ref()?.join(fallback),
                };
                Some(dir.join(file))
            }
        }
    }
}

/// What a file looked like when its derived value was computed.
struct Stamped<T> {
    size: u64,
    modified: Option<SystemTime>,
    value: T,
}

type Memo<T> = Mutex<HashMap<PathBuf, Stamped<T>>>;

fn stamp(path: &Path) -> io::Result<(u64, Option<SystemTime>)> {
    let meta = std::fs::metadata(path)?;
    Ok((meta.len(), meta.modified().ok()))
}

fn remembered<T: Clone>(memo: &Memo<T>, path: &Path, size: u64, modified: Option<SystemTime>) -> Option<T> {
    let memo = memo.lock().expect("not poisoned");
    let seen = memo.get(path)?;
    (seen.size == size && seen.modified == modified).then(|| seen.value.clone())
}

fn remember<T>(memo: &Memo<T>, path: &Path, size: u64, modified: Option<SystemTime>, value: T) {
    memo.lock()
        .expect("not poisoned")
        .insert(path.to_owned(), Stamped { size, modified, value });
}

/// What a toolchain program printed in a root, observed once per generation of
/// the backend that asks (see [`Identities::renewed`]).
#[derive(Clone, PartialEq, Eq, Hash)]
struct Toolchain {
    backend: BackendId,
    program: String,
    args: Vec<String>,
    root: PathBuf,
    generation: u64,
}

impl Kind for Toolchain {
    type Value = String;
    type Error = Infallible;

    fn retention(&self) -> Retention {
        Retention::Sampled
    }

    fn generation(&self) -> Option<(Self, u64)> {
        Some((
            Self {
                generation: 0,
                ..self.clone()
            },
            self.generation,
        ))
    }
}

/// Computes server identities and remembers what it costs to: the hash of each
/// binary and the version it printed.
pub struct Identities {
    user: UserEnv,
    binaries: Memo<Hash>,
    versions: Memo<String>,
    toolchains: Engine<Toolchain>,
    /// How often the servers of each backend were replaced.
    generations: Mutex<HashMap<BackendId, u64>>,
    /// Where files are read and hashed.
    semantic_io: Budget,
    /// Where `--version` and the toolchain are asked.
    probes: Budget,
    /// Tells this run of Vet from every other, for what only this run can vouch for: the schemas
    /// a server loaded from the network.
    session: String,
}

/// Toolchain probes kept.
const TOOLCHAINS_KEPT: usize = 256;

impl Identities {
    pub fn new(user: UserEnv) -> Self {
        Self {
            user,
            binaries: Mutex::default(),
            versions: Mutex::default(),
            toolchains: Engine::new(Limits::entries(TOOLCHAINS_KEPT)),
            generations: Mutex::default(),
            semantic_io: Budget::new(SEMANTIC_IO),
            probes: Budget::new(PROBES),
            session: session(),
        }
    }

    /// Reads files and runs probes on `budgets`.
    pub fn budgeted(mut self, budgets: &Budgets) -> Self {
        self.semantic_io = budgets.semantic_io.clone();
        self.probes = budgets.probes.clone();
        self
    }

    /// Runs `work` on the semantic lane's blocking threads.
    async fn off_thread<T: Send + 'static>(&self, work: impl FnOnce() -> T + Send + 'static) -> T {
        self.semantic_io.blocking(work).await.expect("a blocking task")
    }

    /// The servers of `backend` are replaced: its toolchain is probed anew by the next identity.
    pub fn renewed(&self, backend: &BackendId) {
        *self
            .generations
            .lock()
            .expect("not poisoned")
            .entry(backend.clone())
            .or_default() += 1;
    }

    /// The identity of the server `command` starts for `backend` on `root`.
    pub(super) async fn of(
        &self,
        root: &Path,
        backend: &Backend,
        command: &ServerCommand,
        policy: AnalysisPolicy,
    ) -> Result<ServerIdentity, LaunchError> {
        let config = backend.config();
        let spec = language::by_id(&config.backend.0);
        let named = command.program.clone();
        let program = self
            .off_thread(move || Host::current().find(&named, is_file))
            .await
            .ok_or_else(|| LaunchError::NotInstalled(format!("{} was not found", command.program)))?;
        let binary = self
            .binary_hash(&program)
            .await
            .map_err(|e| LaunchError::Spawn(format!("{}: {e}", program.display())))?;
        let version = match spec {
            Some(spec) => self.version(spec, &program).await,
            None => None,
        };

        let mut digest = Digest::default();
        digest
            .field(&format!("server identity {FORMAT}"))
            .field(&config.backend.0);
        digest
            .field("program")
            .field(&program.to_string_lossy())
            .field(&binary.to_hex())
            .field(version.as_deref().unwrap_or("no version"));
        if let Some(Install::Npm { packages }) = spec.map(|s| s.install) {
            digest.field("package").field(&self.package_manifest(&program).await);
            // The packages installed beside the server answer for it too (the fallback
            // `typescript`, Angular's language service).
            for (package, _) in &packages[1..] {
                digest
                    .field(package)
                    .field(&self.companion_manifest(&program, package).await);
            }
        }
        let setup = setup::setup(config, command, root, SchemaSource::of(policy));
        digest.field("args").field(&setup.args.len().to_string());
        setup.args.iter().for_each(|arg| {
            digest.field(arg);
        });
        let shown = |value: Option<&Value>| value.map_or_else(|| "none".to_owned(), Value::to_string);
        digest
            .field("initialization options")
            .field(&shown(setup.options.as_ref()));
        digest.field("settings").field(&shown(setup.settings.as_ref()));
        digest.field("notifications");
        for (method, params) in &setup.notifications {
            digest.field(method).field(&params.to_string());
        }
        // What a server checks files against is the bundle's content while the network is closed,
        // and whatever the network serves otherwise: that cannot be told, so an answer given
        // online is reused only by servers of the same session of Vet.
        if setup.bundled {
            digest.field("schemas bundled").field(&schemas::fingerprint().to_hex());
        } else if spec.is_some_and(|s| s.schemas != SchemaClient::None) {
            digest.field("schemas online").field(&self.session);
        }
        let capabilities =
            serde_json::to_value(client_capabilities()).map_err(|e| LaunchError::Protocol(e.to_string()))?;
        digest.field("client capabilities").field(&capabilities.to_string());
        self.compiler(&mut digest, command).await?;
        if let Some(spec) = spec {
            self.user_config(&mut digest, &config.backend, root, spec).await;
        }

        let name = spec.map_or(config.binary.as_str(), |s| s.display_name);
        let label = match &version {
            Some(v) if v.starts_with(name) => v.clone(),
            Some(v) => format!("{name} {v}"),
            None => name.to_owned(),
        };
        Ok(ServerIdentity {
            backend: config.backend.clone(),
            fingerprint: digest.finish(),
            label,
        })
    }

    /// The compiler the server is told about: which file, what is in it and which package it belongs to.
    async fn compiler(&self, digest: &mut Digest, command: &ServerCommand) -> Result<(), LaunchError> {
        digest.field("compiler");
        let Some(compiler) = &command.compiler else {
            digest.field("none");
            return Ok(());
        };
        let hash = self
            .binary_hash(compiler)
            .await
            .map_err(|e| LaunchError::Spawn(format!("{}: {e}", compiler.display())))?;
        digest
            .field(&compiler.to_string_lossy())
            .field(&hash.to_hex())
            .field(&self.package_manifest(compiler).await);
        Ok(())
    }

    async fn package_manifest(&self, path: &Path) -> String {
        let path = path.to_owned();
        self.off_thread(move || package_manifest(&path)).await
    }

    /// The `package.json` of the npm package `package` installed in the same `node_modules` as
    /// the one `path` is in.
    async fn companion_manifest(&self, path: &Path, package: &str) -> String {
        let (path, package) = (path.to_owned(), package.to_owned());
        self.off_thread(move || companion_manifest(&path, &package)).await
    }

    /// The toolchain in the root and the configuration outside the repository that the
    /// language's server reads. A file that is not there adds nothing.
    async fn user_config(&self, digest: &mut Digest, backend: &BackendId, root: &Path, spec: &Language) {
        let config = &spec.user_config;
        digest.field("toolchain");
        for (program, args) in config.toolchain {
            let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
            let seen = self.toolchain(backend, program, args, root).await;
            digest.field(program).field(&seen);
        }
        let above: Vec<(String, PathBuf)> = root
            .ancestors()
            .skip(1)
            .enumerate()
            .flat_map(|(level, dir)| {
                config
                    .above_root
                    .iter()
                    .map(move |name| (format!("{level}/{name}"), dir.join(name)))
            })
            .collect();
        let user: Vec<(String, Option<PathBuf>)> = config
            .user_files
            .iter()
            .map(|file| (format!("{file:?}"), self.user.file(file)))
            .collect();
        let (above, user) = self
            .off_thread(move || {
                (
                    hashed(above),
                    hashed(user.into_iter().filter_map(|(k, p)| Some((k, p?)))),
                )
            })
            .await;
        digest.field("above the root");
        for (label, hash) in above {
            digest.field(&label).field(&hash.to_hex());
        }
        digest.field("user files");
        for (label, hash) in user {
            digest.field(&label).field(&hash.to_hex());
        }
        digest.field("environment");
        for name in config.env {
            if let Some(value) = self.user.vars.get(*name) {
                digest.field(name).field(value);
            }
        }
    }

    /// What the toolchain program printed in `root`, probed once per program, root and
    /// generation of the backend, and shared by concurrent callers. A probe that failed
    /// is not remembered: the next one may succeed.
    async fn toolchain(&self, backend: &BackendId, program: &str, args: Vec<String>, root: &Path) -> String {
        let generation = self
            .generations
            .lock()
            .expect("not poisoned")
            .get(backend)
            .copied()
            .unwrap_or(0);
        let key = Toolchain {
            backend: backend.clone(),
            program: program.to_owned(),
            args,
            root: root.to_owned(),
            generation,
        };
        let (probe, probes) = (key.clone(), self.probes.clone());
        let seen = self
            .toolchains
            .get(key, move |_| async move {
                Ok(
                    match capture(&probes, &probe.program, &probe.args, Some(&probe.root)).await {
                        Ok(out) if out.ok => Computed::new(format!("{}{}", out.stdout.trim(), out.stderr.trim()), true),
                        _ => Computed::new("unavailable".to_owned(), false),
                    },
                )
            })
            .await;
        match seen {
            Ok(got) => got.value,
            Err(never) => match never {},
        }
    }

    /// The hash of the file's content, read again only if its size or modification time changed.
    async fn binary_hash(&self, path: &Path) -> io::Result<Hash> {
        let owned = path.to_owned();
        let (size, modified) = self.off_thread(move || stamp(&owned)).await?;
        if let Some(hash) = remembered(&self.binaries, path, size, modified) {
            return Ok(hash);
        }
        let owned = path.to_owned();
        let hash = self
            .off_thread(move || Hash::of_reader(std::fs::File::open(owned)?))
            .await?;
        remember(&self.binaries, path, size, modified, hash);
        Ok(hash)
    }

    /// What `--version` printed, probed again only if the file changed. A probe that
    /// failed is not remembered: the next one may succeed.
    async fn version(&self, spec: &Language, program: &Path) -> Option<String> {
        let owned = program.to_owned();
        let (size, modified) = self.off_thread(move || stamp(&owned)).await.ok()?;
        if let Some(version) = remembered(&self.versions, program, size, modified) {
            return Some(version);
        }
        let version = probe_version(&self.probes, spec, program).await?;
        remember(&self.versions, program, size, modified, version.clone());
        Some(version)
    }
}

/// A name for this run of Vet: when it began and which process it is. Each `Identities` of a
/// process has its own.
fn session() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let began = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "{began}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn hash_of_file(path: &Path) -> Option<Hash> {
    std::fs::read(path).ok().map(|bytes| Hash::of(&bytes))
}

/// The files of `labelled` that exist, with the hash of their content.
fn hashed(labelled: impl IntoIterator<Item = (String, PathBuf)>) -> Vec<(String, Hash)> {
    labelled
        .into_iter()
        .filter_map(|(label, path)| Some((label, hash_of_file(&path)?)))
        .collect()
}

/// The `package.json` of the npm package `path` is in, or is a link into: what says which
/// version of a script's package runs, for a server whose script is the same few lines in all.
fn package_manifest(path: &Path) -> String {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    real.ancestors()
        .skip(1)
        .find_map(|dir| hash_of_file(&dir.join("package.json")))
        .map_or_else(|| "none".to_owned(), |hash| hash.to_hex())
}

/// The `package.json` of `package` in the `node_modules` that `path` is in, or `none`.
fn companion_manifest(path: &Path, package: &str) -> String {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    real.ancestors()
        .skip(1)
        .filter(|dir| dir.file_name().is_some_and(|name| name == "node_modules"))
        .find_map(|dir| hash_of_file(&dir.join(package).join("package.json")))
        .map_or_else(|| "none".to_owned(), |hash| hash.to_hex())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;

    /// A server executable whose `--version` is the content of `version.txt` beside it.
    struct Setup {
        dir: TempDir,
        /// Home, the user's files and the root's parent, apart from the program.
        world: TempDir,
        vars: HashMap<String, String>,
        args: Vec<String>,
    }

    const SCRIPT: &str = "#!/bin/sh\necho \"$(cat \"$(dirname \"$0\")/version.txt\")\"\n# a\n";

    impl Setup {
        fn new() -> Self {
            let setup = Self {
                dir: tempfile::tempdir().unwrap(),
                world: tempfile::tempdir().unwrap(),
                vars: HashMap::new(),
                args: Vec::new(),
            };
            std::fs::write(setup.dir.path().join("version.txt"), "rust-analyzer 1.0").unwrap();
            setup.write_program(SCRIPT);
            std::fs::create_dir_all(setup.root()).unwrap();
            setup
        }

        fn program(&self) -> PathBuf {
            self.dir.path().join("rust-analyzer")
        }

        fn write_program(&self, script: &str) {
            let program = self.program();
            std::fs::write(&program, script).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            // A process forked by another test while this was written still holds the
            // file open for writing, and running it fails ("text file busy") until it execs.
            for _ in 0..200 {
                if std::process::Command::new(&program).output().is_ok() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            panic!("{} cannot be run", program.display());
        }

        /// Two levels below `world`, so there are directories above it that are ours.
        fn root(&self) -> PathBuf {
            self.world.path().join("up/project")
        }

        fn put(&self, relative: &str, text: &str) {
            let path = self.world.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        fn backend(&self) -> Backend {
            let Backend::GenericLsp(mut config) = language::RUST.backend() else {
                unreachable!("Rust is a generic server")
            };
            config.args.clone_from(&self.args);
            Backend::GenericLsp(config)
        }

        async fn identity(&self, identities: &Identities) -> ServerIdentity {
            let command = ServerCommand {
                program: self.program().to_string_lossy().into_owned(),
                compiler: None,
            };
            identities
                .of(&self.root(), &self.backend(), &command, AnalysisPolicy::EXECUTING)
                .await
                .unwrap()
        }

        /// The fingerprint of `language`'s server, which is the program of this setup.
        async fn fingerprint_of(&self, identities: &Identities, language: &Language, policy: AnalysisPolicy) -> Hash {
            let command = ServerCommand {
                program: self.program().to_string_lossy().into_owned(),
                compiler: None,
            };
            identities
                .of(&self.root(), &language.backend(), &command, policy)
                .await
                .unwrap()
                .fingerprint
        }

        fn identities(&self) -> Identities {
            Identities::new(UserEnv {
                home: Some(self.world.path().join("home")),
                vars: self.vars.clone(),
            })
        }

        async fn fingerprint(&self) -> Hash {
            self.identity(&self.identities()).await.fingerprint
        }
    }

    #[tokio::test]
    async fn each_input_on_its_own_changes_the_fingerprint_and_nothing_else_does() {
        let mut setup = Setup::new();
        let base = setup.fingerprint().await;
        assert_eq!(setup.fingerprint().await, base, "the same inputs");

        // What changes it, one thing at a time; each is undone before the next.
        setup.write_program(&SCRIPT.replace("# a", "# a longer comment"));
        assert_ne!(setup.fingerprint().await, base, "binary");
        setup.write_program(SCRIPT);
        assert_eq!(
            setup.fingerprint().await,
            base,
            "a binary written again with the same content"
        );

        let version = setup.dir.path().join("version.txt");
        std::fs::write(&version, "rust-analyzer 2.0").unwrap();
        assert_ne!(setup.fingerprint().await, base, "version");
        std::fs::write(&version, "rust-analyzer 1.0").unwrap();

        setup.args = vec!["--log".into()];
        assert_ne!(setup.fingerprint().await, base, "arguments");
        setup.args.clear();

        setup.vars.insert("RUSTUP_TOOLCHAIN".into(), "nightly".into());
        assert_ne!(setup.fingerprint().await, base, "listed environment variable");
        setup.vars.clear();

        setup.put("home/.cargo/config.toml", "[source]\n");
        let with_config = setup.fingerprint().await;
        assert_ne!(with_config, base, "user-level cargo config");
        setup.put("home/.cargo/config.toml", "[source.crates-io]\n");
        assert_ne!(setup.fingerprint().await, with_config, "its content");
        std::fs::remove_file(setup.world.path().join("home/.cargo/config.toml")).unwrap();

        setup.put("home/.config/rust-analyzer/rust-analyzer.toml", "x = 1\n");
        assert_ne!(setup.fingerprint().await, base, "user-level server settings");
        std::fs::remove_file(setup.world.path().join("home/.config/rust-analyzer/rust-analyzer.toml")).unwrap();

        setup.put("up/.cargo/config.toml", "[build]\n");
        assert_ne!(setup.fingerprint().await, base, "cargo config above the root");
        std::fs::remove_file(setup.world.path().join("up/.cargo/config.toml")).unwrap();

        // What does not.
        setup.vars.insert("NOT_READ_BY_ANYONE".into(), "1".into());
        setup.put("home/notes.txt", "unrelated");
        setup.put("up/project/.cargo/config.toml", "[build]\n");
        setup.put("up/project/Cargo.toml", "[package]\n");
        assert_eq!(
            setup.fingerprint().await,
            base,
            "unlisted inputs and the repository's own files"
        );
    }

    #[tokio::test]
    async fn the_compiler_the_server_is_told_about_is_part_of_the_identity() {
        let setup = Setup::new();
        let tsserver = setup.dir.path().join("tsserver.js");
        std::fs::write(&tsserver, "one").unwrap();
        let identities = setup.identities();
        let identify = |compiler: Option<PathBuf>| {
            let command = ServerCommand {
                program: setup.program().to_string_lossy().into_owned(),
                compiler,
            };
            let backend = language::TYPESCRIPT.backend();
            let (identities, root) = (&identities, setup.root());
            async move {
                identities
                    .of(&root, &backend, &command, AnalysisPolicy::EXECUTING)
                    .await
                    .unwrap()
                    .fingerprint
            }
        };
        let none = identify(None).await;
        let one = identify(Some(tsserver.clone())).await;
        assert_ne!(one, none);
        std::fs::write(&tsserver, "two").unwrap();
        assert_ne!(identify(Some(tsserver)).await, one);
    }

    #[tokio::test]
    async fn where_schemas_may_come_from_is_part_of_the_identity_of_a_server_that_uses_them() {
        let setup = Setup::new();
        for language in [&language::YAML, &language::JSON] {
            // The version a program prints is remembered by its path, so each language has its own run.
            let identities = setup.identities();
            let closed = setup
                .fingerprint_of(&identities, language, AnalysisPolicy::LIMITED)
                .await;
            assert_eq!(
                setup
                    .fingerprint_of(&identities, language, AnalysisPolicy::SERVERS)
                    .await,
                closed,
                "{}: policies that keep the network closed share an identity",
                language.id
            );
            let open = setup
                .fingerprint_of(&identities, language, AnalysisPolicy::EXECUTING)
                .await;
            assert_ne!(open, closed, "{}: bundled and online answers never mix", language.id);
            assert_eq!(
                setup
                    .fingerprint_of(&identities, language, AnalysisPolicy::EXECUTING)
                    .await,
                open,
                "{}: within a run of Vet an online identity is stable",
                language.id
            );
            let next_run = setup.identities();
            assert_ne!(
                setup
                    .fingerprint_of(&next_run, language, AnalysisPolicy::EXECUTING)
                    .await,
                open,
                "{}: another run cannot vouch for what the network served",
                language.id
            );
            assert_eq!(
                setup.fingerprint_of(&next_run, language, AnalysisPolicy::LIMITED).await,
                closed,
                "{}: the bundle's answers are the same in every run",
                language.id
            );
        }
        // A language that checks nothing against schemas is the same either way.
        let identities = setup.identities();
        let limited = setup
            .fingerprint_of(&identities, &language::RUST, AnalysisPolicy::LIMITED)
            .await;
        let executing = setup
            .fingerprint_of(&identities, &language::RUST, AnalysisPolicy::EXECUTING)
            .await;
        assert_eq!(limited, executing);
    }

    #[tokio::test]
    async fn an_unchanged_binary_is_not_read_again_and_a_changed_one_is() {
        let setup = Setup::new();
        let identities = setup.identities();
        let first = setup.identity(&identities).await;
        assert_eq!(first.label, "rust-analyzer 1.0");
        let stamp = std::fs::metadata(setup.program()).unwrap().modified().unwrap();

        // Other content of the same size under the same modification time: only a
        // memo that is not consulted would notice it. The version it prints is not asked for again either.
        setup.write_program(&SCRIPT.replace("# a", "# b"));
        std::fs::write(setup.dir.path().join("version.txt"), "rust-analyzer 2.0").unwrap();
        let program = std::fs::File::options().write(true).open(setup.program()).unwrap();
        program.set_modified(stamp).unwrap();
        let unchanged = setup.identity(&identities).await;
        assert_eq!(unchanged.fingerprint, first.fingerprint);
        assert_eq!(unchanged.label, "rust-analyzer 1.0");
        assert_ne!(setup.fingerprint().await, first.fingerprint, "a fresh memo reads it");

        // Its modification time moves: it is read again, and the version asked for again.
        program.set_modified(stamp + Duration::from_secs(5)).unwrap();
        let again = setup.identity(&identities).await;
        assert_ne!(again.fingerprint, first.fingerprint);
        assert_eq!(again.label, "rust-analyzer 2.0");
    }

    #[tokio::test]
    async fn a_toolchain_is_probed_once_per_generation_of_its_backend_however_many_ask() {
        let setup = Setup::new();
        let tool = setup.dir.path().join("tool");
        std::fs::write(
            &tool,
            "#!/bin/sh\necho run >> \"$(dirname \"$0\")/runs\"\ncat \"$(dirname \"$0\")/printed\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let printed = setup.dir.path().join("printed");
        std::fs::write(&printed, "one").unwrap();
        let runs = || std::fs::read_to_string(setup.dir.path().join("runs")).map_or(0, |runs| runs.lines().count());

        let identities = setup.identities();
        let (rust, python) = (BackendId("rust".into()), BackendId("python".into()));
        let program = tool.to_string_lossy().into_owned();
        let root = setup.root();
        let probe = || async { identities.toolchain(&rust, &program, vec![], &root).await };

        let seen = futures::future::join_all((0..8).map(|_| probe())).await;
        assert!(seen.iter().all(|printed| printed == "one"), "{seen:?}");
        assert_eq!(runs(), 1);

        std::fs::write(&printed, "two").unwrap();
        assert_eq!(probe().await, "one", "the same generation");
        identities.renewed(&python);
        assert_eq!(probe().await, "one", "another backend's generation");
        assert_eq!(runs(), 1);
        identities.renewed(&rust);
        assert_eq!(probe().await, "two", "a new generation");
        assert_eq!(runs(), 2);
    }
}
