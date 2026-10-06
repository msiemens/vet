//! Finding and invoking programs: `PATH` search, `PATHEXT` on Windows and the
//! `cmd /c` wrapper for `.cmd` and `.bat` shims. Every spawn of an external
//! tool goes through [`invocation`], so none of this is repeated elsewhere.

use std::path::{Path, PathBuf};

/// What a program search depends on. Explicit so the rules are testable on any OS.
#[derive(Clone, Debug)]
pub struct Host {
    pub windows: bool,
    /// The directories of `PATH`.
    pub path: Vec<PathBuf>,
    /// The extensions of `PATHEXT`, with their dots; unused unless `windows`.
    pub pathext: Vec<String>,
}

/// `PATHEXT` when the variable is unset, as `cmd.exe` documents it.
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

impl Host {
    pub fn current() -> Self {
        Self {
            windows: cfg!(windows),
            path: std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default(),
            pathext: parse_pathext(std::env::var("PATHEXT").ok().as_deref()),
        }
    }

    /// The names `name` can be run as: itself on Unix, and on Windows itself if
    /// it already ends in a `PATHEXT` extension, else with each one appended.
    fn candidates(&self, name: &str) -> Vec<String> {
        if !self.windows {
            return vec![name.to_owned()];
        }
        let lower = name.to_ascii_lowercase();
        if self.pathext.iter().any(|e| lower.ends_with(&e.to_ascii_lowercase())) {
            return vec![name.to_owned()];
        }
        self.pathext.iter().map(|e| format!("{name}{e}")).collect()
    }

    /// The file `name` runs as. A name with a directory part is not searched for.
    pub fn find(&self, name: &str, is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
        let candidates = self.candidates(name);
        if name.contains('/') || (self.windows && name.contains('\\')) {
            return candidates.iter().map(PathBuf::from).find(|p| is_file(p));
        }
        self.path
            .iter()
            .flat_map(|dir| candidates.iter().map(move |c| dir.join(c)))
            .find(|p| is_file(p))
    }

    /// The same, in `dir` alone: for tools installed into a known directory.
    pub fn find_in(&self, dir: &Path, name: &str, is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
        self.candidates(name).iter().map(|c| dir.join(c)).find(|p| is_file(p))
    }

    /// The program and arguments that run `program`: a Windows `.cmd` or `.bat`
    /// shim cannot be started directly and goes through `cmd /c`.
    pub fn wrap(&self, program: &Path, args: &[String]) -> (String, Vec<String>) {
        let shim = self.windows
            && program
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
        let program = program.to_string_lossy().into_owned();
        if shim {
            let mut wrapped = vec!["/c".to_owned(), program];
            wrapped.extend_from_slice(args);
            ("cmd".to_owned(), wrapped)
        } else {
            (program, args.to_vec())
        }
    }
}

pub fn parse_pathext(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or(DEFAULT_PATHEXT)
        .split(';')
        .filter(|e| !e.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn is_file(path: &Path) -> bool {
    path.is_file()
}

/// The program and arguments to spawn for `program`, found on this host's
/// `PATH` if it is a bare name. A name that is not found is left as it is, so
/// the spawn reports it.
pub fn invocation(program: &str, args: &[String]) -> (String, Vec<String>) {
    let host = Host::current();
    let found = host.find(program, is_file).unwrap_or_else(|| PathBuf::from(program));
    host.wrap(&found, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows() -> Host {
        Host {
            windows: true,
            path: vec![PathBuf::from("/c/bin"), PathBuf::from("/c/npm")],
            pathext: parse_pathext(Some(".COM;.EXE;.BAT;.CMD")),
        }
    }

    fn unix() -> Host {
        Host {
            windows: false,
            path: vec![PathBuf::from("/usr/bin"), PathBuf::from("/opt/bin")],
            pathext: Vec::new(),
        }
    }

    fn present<'a>(files: &'a [&str]) -> impl Fn(&Path) -> bool + 'a {
        move |p| files.iter().any(|f| Path::new(f) == p)
    }

    #[test]
    fn pathext_defaults_and_skips_empty_entries() {
        assert_eq!(parse_pathext(None), [".COM", ".EXE", ".BAT", ".CMD"]);
        assert_eq!(parse_pathext(Some(".EXE;;.CMD;")), [".EXE", ".CMD"]);
    }

    #[test]
    fn windows_appends_extensions_in_pathext_order() {
        let host = windows();
        let files = ["/c/npm/tsc.CMD", "/c/bin/tsc.BAT"];
        // The earlier directory wins before a later one, whatever the extension.
        let found = host.find("tsc", present(&files)).unwrap();
        assert_eq!(found, Path::new("/c/bin/tsc.BAT"));
        let found = host
            .find("tsc", present(&["/c/bin/tsc.CMD", "/c/bin/tsc.EXE"]))
            .unwrap();
        assert_eq!(found, Path::new("/c/bin/tsc.EXE"));
    }

    #[test]
    fn windows_does_not_run_an_extensionless_file() {
        assert_eq!(windows().find("npm", present(&["/c/bin/npm"])), None);
    }

    #[test]
    fn a_name_with_a_pathext_extension_is_tried_as_given() {
        let found = windows().find("npm.cmd", present(&["/c/npm/npm.cmd"]));
        assert_eq!(found, Some(PathBuf::from("/c/npm/npm.cmd")));
        let found = windows().find("NPM.CMD", present(&["/c/npm/NPM.CMD"]));
        assert!(found.is_some());
    }

    #[test]
    fn a_name_with_a_directory_part_is_not_searched_for() {
        let host = unix();
        assert_eq!(
            host.find("./bin/tool", present(&["./bin/tool", "/usr/bin/./bin/tool"])),
            Some(PathBuf::from("./bin/tool"))
        );
        assert_eq!(host.find("./bin/tool", present(&["/usr/bin/tool"])), None);
    }

    #[test]
    fn unix_takes_the_name_as_is() {
        assert_eq!(
            unix().find("rust-analyzer", present(&["/opt/bin/rust-analyzer"])),
            Some(PathBuf::from("/opt/bin/rust-analyzer"))
        );
        assert_eq!(
            unix().find("rust-analyzer.exe", present(&["/opt/bin/rust-analyzer"])),
            None
        );
    }

    #[test]
    fn find_in_looks_in_one_directory() {
        let files = ["/c/vet/node_modules/.bin/tsserver.CMD"];
        let dir = Path::new("/c/vet/node_modules/.bin");
        assert_eq!(
            windows().find_in(dir, "tsserver", present(&files)),
            Some(dir.join("tsserver.CMD"))
        );
    }

    #[test]
    fn windows_shims_run_through_cmd() {
        let host = windows();
        let args = vec!["--stdio".to_owned()];
        let (program, wrapped) = host.wrap(Path::new("C:\\npm\\typescript-language-server.cmd"), &args);
        assert_eq!(program, "cmd");
        assert_eq!(wrapped, ["/c", "C:\\npm\\typescript-language-server.cmd", "--stdio"]);
        let (program, wrapped) = host.wrap(Path::new("C:\\x\\Old.BAT"), &[]);
        assert_eq!((program.as_str(), wrapped.len()), ("cmd", 2));
    }

    #[test]
    fn executables_and_unix_scripts_are_not_wrapped() {
        let args = vec!["a".to_owned()];
        let (program, wrapped) = windows().wrap(Path::new("C:\\bin\\rust-analyzer.exe"), &args);
        assert_eq!(
            (program.as_str(), wrapped),
            ("C:\\bin\\rust-analyzer.exe", args.clone())
        );
        let (program, wrapped) = unix().wrap(Path::new("/x/tool.cmd"), &args);
        assert_eq!((program.as_str(), wrapped), ("/x/tool.cmd", args));
    }
}
