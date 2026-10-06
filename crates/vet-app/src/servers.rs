//! Language servers on this machine (§7.1): what each language resolves to,
//! the user's custom paths, and installing the pinned servers.

use std::fmt;
use std::path::Path;

use tokio::sync::Mutex;
use vet_semantic::ports::{BackendId, ServerConfig};

use crate::ports::{ServerCommand, ServerLocator, ServerResolver, ServerSettings, ServerState, StoreError};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ServerStatus {
    pub language: BackendId,
    pub state: ServerState,
    pub custom_path: Option<String>,
    /// The version `install` provides; `None` if Vet cannot install this language's server.
    pub pinned: Option<String>,
    /// Why installing is not possible right now.
    pub install_blocker: Option<String>,
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
}

impl<R: ServerResolver + Sync, S: ServerSettings + Sync> Servers<R, S> {
    pub fn new(resolver: R, settings: S, languages: Vec<BackendId>) -> Self {
        Self {
            resolver,
            settings,
            languages,
            installing: Mutex::new(()),
        }
    }

    fn known(&self, language: &str) -> Result<&BackendId, ServersError> {
        self.languages
            .iter()
            .find(|l| l.0 == language)
            .ok_or_else(|| ServersError::UnknownLanguage(language.to_owned()))
    }

    /// Every language, as it resolves for the project at `project`.
    pub async fn status(&self, project: &Path) -> Result<Vec<ServerStatus>, ServersError> {
        let mut all = Vec::new();
        for language in &self.languages {
            let custom_path = self.settings.custom_server_path(language.0.clone()).await?;
            let state = self.resolver.resolve(language, project, custom_path.as_deref()).await;
            all.push(ServerStatus {
                language: language.clone(),
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
        Ok(self.settings.set_custom_server_path(language.0.clone(), path).await?)
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
            .settings
            .custom_server_path(language.0.clone())
            .await
            .map_err(|e| format!("cannot read the language server settings: {}", e.0))?;
        match self.resolver.resolve(language, project, custom.as_deref()).await {
            ServerState::Found(found) => Ok(found.command),
            ServerState::Missing { reason } => Err(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    use super::*;
    use crate::ports::{FoundServer, ServerOrigin};

    #[derive(Default)]
    struct Settings(StdMutex<HashMap<String, String>>);

    impl ServerSettings for Settings {
        async fn custom_server_path(&self, language: String) -> Result<Option<String>, StoreError> {
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

    /// Finds a server only where the custom path points.
    struct Resolver;

    impl ServerResolver for Resolver {
        async fn resolve(&self, language: &BackendId, _project: &Path, custom: Option<&str>) -> ServerState {
            match custom {
                Some(path) => ServerState::Found(FoundServer {
                    command: ServerCommand {
                        program: path.into(),
                        initialization_options: None,
                    },
                    origin: ServerOrigin::Custom,
                    version: None,
                    notes: Vec::new(),
                }),
                None => ServerState::Missing {
                    reason: format!("no {} server", language.0),
                },
            }
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
        Servers::new(Resolver, Settings::default(), languages)
    }

    fn config(language: &str) -> ServerConfig {
        ServerConfig {
            backend: BackendId(language.into()),
            binary: String::new(),
            args: vec![],
            filetypes: vec![],
            root_markers: vec![],
            executes_project_code_during_analysis: false,
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
    async fn status_reports_pins_and_install_blockers() {
        let status = servers().status(Path::new("/p")).await.unwrap();
        let pinned: Vec<_> = status.iter().map(|s| s.pinned.clone()).collect();
        assert_eq!(pinned, [None, Some("1.0.0".into()), None]);
        assert_eq!(status[2].install_blocker.as_deref(), Some("npm is missing"));
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
