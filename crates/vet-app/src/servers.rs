//! Language servers on this machine (§7.1): what each language resolves to,
//! the user's custom paths, and installing the pinned servers.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use tokio::sync::Mutex;
use vet_semantic::ports::{BackendId, ServerConfig};

use crate::ports::{ServerCommand, ServerLocator, ServerResolver, ServerSettings, ServerState, StoreError};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ServerStatus {
    pub language: BackendId,
    /// The language's name as a person knows it, e.g. "C#".
    pub title: String,
    /// The server's name as a person knows it, e.g. "Pyright".
    pub server: String,
    /// The file extensions (without the dot) the server serves.
    pub extensions: Vec<String>,
    /// The names of files without an extension that it serves.
    pub file_names: Vec<String>,
    pub state: ServerState,
    pub custom_path: Option<String>,
    /// The version `install` provides; `None` if Vet cannot install this language's server.
    pub pinned: Option<String>,
    /// Why installing is not possible right now.
    pub install_blocker: Option<String>,
}

impl ServerStatus {
    /// Whether `install` can run now: there is a server to install and nothing in the way.
    pub fn can_install(&self) -> bool {
        self.pinned.is_some() && self.install_blocker.is_none()
    }
}

#[derive(Debug)]
pub enum ServersError {
    UnknownLanguage(String),
    Store(StoreError),
    /// Fit to show.
    Install(String),
}

impl fmt::Display for ServersError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLanguage(language) => write!(f, "unknown language {language}"),
            Self::Store(e) => write!(f, "store: {}", e.0),
            Self::Install(why) => f.write_str(why),
        }
    }
}

impl From<StoreError> for ServersError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

pub struct Servers<R, S> {
    resolver: R,
    settings: S,
    languages: Vec<BackendId>,
    /// One install at a time: they share the data directory.
    installing: Mutex<()>,
    /// The custom path of each language, read from `settings` once and kept current by
    /// `set_custom_path`, so that locating a server does not wait for the store, which
    /// the review's writes share. The lock is held across a read or a write of the store,
    /// so a read cannot overwrite a newer write.
    custom_paths: Mutex<HashMap<BackendId, Option<String>>>,
}

impl<R: ServerResolver + Sync, S: ServerSettings + Sync> Servers<R, S> {
    pub fn new(resolver: R, settings: S, languages: Vec<BackendId>) -> Self {
        Self {
            resolver,
            settings,
            languages,
            installing: Mutex::new(()),
            custom_paths: Mutex::new(HashMap::new()),
        }
    }

    async fn custom_path(&self, language: &BackendId) -> Result<Option<String>, StoreError> {
        let mut paths = self.custom_paths.lock().await;
        if let Some(path) = paths.get(language) {
            return Ok(path.clone());
        }
        let path = self.settings.custom_server_path(language.0.clone()).await?;
        paths.insert(language.clone(), path.clone());
        Ok(path)
    }

    /// The language by the name `status` gives it.
    pub fn known(&self, language: &str) -> Result<&BackendId, ServersError> {
        self.languages
            .iter()
            .find(|l| l.0 == language)
            .ok_or_else(|| ServersError::UnknownLanguage(language.to_owned()))
    }

    /// Every language, as it resolves for the project at `project`.
    pub async fn status(&self, project: &Path) -> Result<Vec<ServerStatus>, ServersError> {
        let mut all = Vec::new();
        for language in &self.languages {
            let custom_path = self.custom_path(language).await?;
            let mut state = self.resolver.resolve(language, project, custom_path.as_deref()).await;
            if let ServerState::Found(found) = &mut state {
                found.version = self.resolver.version(language, found).await;
            }
            all.push(ServerStatus {
                language: language.clone(),
                title: self.resolver.title(language),
                server: self.resolver.display_name(language),
                extensions: self.resolver.extensions(language),
                file_names: self.resolver.file_names(language),
                state,
                custom_path,
                pinned: self.resolver.pinned(language),
                install_blocker: self.resolver.install_blocker(language),
            });
        }
        Ok(all)
    }

    /// Stores the path of the server to use for `language`; `None` or a blank
    /// path removes it. The path is not checked: `status` reports whether it works.
    pub async fn set_custom_path(&self, language: &str, path: Option<String>) -> Result<(), ServersError> {
        let language = self.known(language)?;
        let path = path.map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
        let mut paths = self.custom_paths.lock().await;
        self.settings
            .set_custom_server_path(language.0.clone(), path.clone())
            .await?;
        paths.insert(language.clone(), path);
        Ok(())
    }

    /// Installs the pinned server of `language`; `progress` gets the tool's output.
    pub async fn install(&self, language: &str, progress: &(dyn Fn(&str) + Send + Sync)) -> Result<(), ServersError> {
        let language = self.known(language)?;
        let _one_at_a_time = self.installing.lock().await;
        if let Some(why) = self.resolver.install_blocker(language) {
            return Err(ServersError::Install(why));
        }
        self.resolver
            .install(language, progress)
            .await
            .map_err(ServersError::Install)
    }
}

impl<R: ServerResolver + Sync, S: ServerSettings + Sync> ServerLocator for Servers<R, S> {
    async fn locate(&self, config: &ServerConfig, project: &Path) -> Result<ServerCommand, String> {
        let language = &config.backend;
        let custom = self
            .custom_path(language)
            .await
            .map_err(|e| format!("cannot read the language server settings: {}", e.0))?;
        match self.resolver.resolve(language, project, custom.as_deref()).await {
            ServerState::Found(found) => Ok(found.command),
            ServerState::Missing { reason } => Err(reason),
        }
    }
}

impl<T: ServerLocator + Send + Sync> ServerLocator for std::sync::Arc<T> {
    fn locate(
        &self,
        config: &ServerConfig,
        project: &Path,
    ) -> impl Future<Output = Result<ServerCommand, String>> + Send {
        T::locate(self, config, project)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    use super::*;
    use crate::ports::{FoundServer, ServerOrigin};

    /// The stored paths, and how often they were read.
    #[derive(Default)]
    struct Settings(StdMutex<HashMap<String, String>>, std::sync::atomic::AtomicUsize);

    impl ServerSettings for Settings {
        async fn custom_server_path(&self, language: String) -> Result<Option<String>, StoreError> {
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.0.lock().unwrap().get(&language).cloned())
        }

        async fn set_custom_server_path(&self, language: String, path: Option<String>) -> Result<(), StoreError> {
            let mut all = self.0.lock().unwrap();
            match path {
                Some(path) => all.insert(language, path),
                None => all.remove(&language),
            };
            Ok(())
        }
    }

    /// Finds a server only where the custom path points. Counts the version lookups.
    #[derive(Default)]
    struct Resolver(std::sync::atomic::AtomicUsize);

    impl ServerResolver for Resolver {
        async fn resolve(&self, language: &BackendId, _project: &Path, custom: Option<&str>) -> ServerState {
            match custom {
                Some(path) => ServerState::Found(FoundServer {
                    command: ServerCommand {
                        program: path.into(),
                        compiler: None,
                    },
                    origin: ServerOrigin::Custom,
                    version: None,
                    tsserver: None,
                }),
                None => ServerState::Missing {
                    reason: format!("no {} server", language.0),
                },
            }
        }

        async fn version(&self, language: &BackendId, _: &FoundServer) -> Option<String> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(format!("{}-1.2", language.0))
        }

        fn title(&self, language: &BackendId) -> String {
            language.0.to_uppercase()
        }

        fn display_name(&self, language: &BackendId) -> String {
            format!("{}-server", language.0)
        }

        fn extensions(&self, language: &BackendId) -> Vec<String> {
            vec![language.0.clone()]
        }

        fn file_names(&self, _: &BackendId) -> Vec<String> {
            vec![]
        }

        fn pinned(&self, language: &BackendId) -> Option<String> {
            (language.0 == "typescript").then(|| "1.0.0".into())
        }

        fn install_blocker(&self, language: &BackendId) -> Option<String> {
            (language.0 == "python").then(|| "npm is missing".into())
        }

        async fn install(&self, _: &BackendId, progress: &(dyn Fn(&str) + Send + Sync)) -> Result<(), String> {
            progress("installing");
            Ok(())
        }
    }

    fn servers() -> Servers<Resolver, Settings> {
        let languages = ["rust", "typescript", "python"].map(|l| BackendId(l.into())).to_vec();
        Servers::new(Resolver::default(), Settings::default(), languages)
    }

    #[tokio::test]
    async fn the_version_is_learned_for_the_status_but_not_when_locating_for_a_start() {
        let servers = servers();
        servers.set_custom_path("rust", Some("/opt/ra".into())).await.unwrap();
        servers.locate(&config("rust"), Path::new("/p")).await.unwrap();
        assert_eq!(servers.resolver.0.load(std::sync::atomic::Ordering::SeqCst), 0);

        let status = servers.status(Path::new("/p")).await.unwrap();
        let ServerState::Found(found) = &status[0].state else {
            panic!("{:?}", status[0].state)
        };
        assert_eq!(found.version.as_deref(), Some("rust-1.2"));
        assert_eq!(servers.resolver.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    fn config(language: &str) -> ServerConfig {
        ServerConfig {
            backend: BackendId(language.into()),
            binary: String::new(),
            args: vec![],
            filetypes: vec![],
            file_names: vec![],
            requires: vec![],
            root_markers: vec![],
            executes_project_code_during_analysis: false,
            loads_project_on_open: false,
        }
    }

    #[tokio::test]
    async fn a_custom_path_is_used_by_status_and_by_the_launcher_lookup() {
        let servers = servers();
        let project = Path::new("/p");
        assert!(
            servers
                .locate(&config("rust"), project)
                .await
                .unwrap_err()
                .contains("no rust")
        );

        servers
            .set_custom_path("rust", Some("  /opt/ra  ".into()))
            .await
            .unwrap();
        let located = servers.locate(&config("rust"), project).await.unwrap();
        assert_eq!(located.program, "/opt/ra");
        let status = servers.status(project).await.unwrap();
        assert_eq!(status[0].custom_path.as_deref(), Some("/opt/ra"));
        assert!(matches!(status[1].state, ServerState::Missing { .. }));

        servers.set_custom_path("rust", Some("   ".into())).await.unwrap();
        assert!(servers.locate(&config("rust"), project).await.is_err());
    }

    #[tokio::test]
    async fn locating_servers_reads_the_store_once_per_language_and_sees_every_change() {
        let servers = servers();
        let project = Path::new("/p");
        for _ in 0..5 {
            assert!(servers.locate(&config("rust"), project).await.is_err());
        }
        assert_eq!(servers.settings.1.load(std::sync::atomic::Ordering::SeqCst), 1);

        servers.set_custom_path("rust", Some("/opt/ra".into())).await.unwrap();
        for _ in 0..5 {
            assert_eq!(
                servers.locate(&config("rust"), project).await.unwrap().program,
                "/opt/ra"
            );
        }
        servers.status(project).await.unwrap();
        assert_eq!(servers.settings.1.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn only_a_pinned_server_without_a_blocker_can_be_installed() {
        let status = servers().status(Path::new("/p")).await.unwrap();
        // Rust has no pin; Python has one but npm is missing; TypeScript has one and nothing in the way.
        let can_install: Vec<_> = status.iter().map(ServerStatus::can_install).collect();
        assert_eq!(can_install, [false, true, false]);
    }

    #[tokio::test]
    async fn unknown_languages_and_blocked_installs_are_refused() {
        let servers = servers();
        assert!(matches!(
            servers.set_custom_path("cobol", Some("x".into())).await,
            Err(ServersError::UnknownLanguage(_))
        ));
        let progress = |_: &str| {};
        assert!(matches!(
            servers.install("python", &progress).await,
            Err(ServersError::Install(why)) if why == "npm is missing"
        ));
        servers.install("typescript", &progress).await.unwrap();
    }
}
