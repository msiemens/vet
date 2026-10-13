//! What a server is told besides the protocol, and how it is answered: its arguments, the
//! options of `initialize`, the settings it pulls and is sent, and where the schemas it asks for
//! come from. All of it shapes the server's answers, so all of it is part of its identity (I2).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vet_app::ports::ServerCommand;
use vet_semantic::ports::ServerConfig;

use crate::exec::{Host, is_file};
use crate::language::{self, Compiler, SchemaClient};
use crate::schemas::{self, SchemaSource};

#[derive(Debug, PartialEq)]
pub(super) struct Setup {
    pub args: Vec<String>,
    /// `initializationOptions`.
    pub options: Option<Value>,
    /// By section (`yaml`, `json`): what the server's `workspace/configuration` requests are
    /// answered with, and what `workspace/didChangeConfiguration` carries after `initialized`.
    pub settings: Option<Value>,
    /// Sent after `initialized`, in order.
    pub notifications: Vec<(&'static str, Value)>,
    /// The server asks for its settings after `initialized`, and what it publishes before it has
    /// them is not what it would publish with them.
    pub pulls_settings: bool,
    /// The server asks Vet which schemas a file has (`custom/schema/request`), which Vet answers
    /// by the file's name.
    pub associates_schemas: bool,
    /// The server's requests for a schema by address are answered from the bundle (see
    /// [`crate::schemas`]) and not by the network.
    pub bundled: bool,
}

/// The nearest `node_modules` directory above `program`, once links are resolved: where the
/// packages installed beside a server are.
fn modules_of(program: &str) -> Option<PathBuf> {
    let found = Host::current().find(program, is_file)?;
    let real = std::fs::canonicalize(found).ok()?;
    real.ancestors()
        .skip(1)
        .find(|dir| dir.file_name().is_some_and(|name| name == "node_modules"))
        .map(Path::to_owned)
}

/// The directories to look for `typescript` and `@angular/language-service` in: the project's
/// own packages first, then the server's.
fn angular_probes(root: &Path, command: &ServerCommand) -> (String, String) {
    let join = |dirs: Vec<PathBuf>| {
        dirs.iter()
            .map(|d| d.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(",")
    };
    let own = modules_of(&command.program);
    let project = root.join("node_modules");
    // `<package>/lib/tsserver.js`: the directory of packages the compiler is in.
    let compiler = command
        .compiler
        .as_deref()
        .and_then(|c| c.ancestors().nth(3))
        .map(Path::to_owned);
    let typescript = join(compiler.into_iter().chain(own.clone()).collect());
    let angular = join(std::iter::once(project).chain(own).collect());
    (typescript, angular)
}

pub(super) fn setup(config: &ServerConfig, command: &ServerCommand, root: &Path, source: SchemaSource) -> Setup {
    let mut setup = Setup {
        args: config.args.clone(),
        options: None,
        settings: None,
        notifications: Vec::new(),
        pulls_settings: false,
        associates_schemas: false,
        bundled: false,
    };
    let Some(language) = language::by_id(&config.backend.0) else {
        return setup;
    };
    if language.compiler == Compiler::TypeScript {
        match language.id {
            "typescript" => {
                setup.options = command
                    .compiler
                    .as_ref()
                    .map(|compiler| json!({"tsserver": {"path": compiler.to_string_lossy()}}));
            }
            "angular" => {
                let (typescript, angular) = angular_probes(root, command);
                setup.args.extend([
                    "--tsProbeLocations".to_owned(),
                    typescript,
                    "--ngProbeLocations".to_owned(),
                    angular,
                ]);
            }
            _ => {}
        }
    }
    let closed = source == SchemaSource::Bundled;
    match language.schemas {
        SchemaClient::None => {}
        SchemaClient::Yaml => {
            setup.settings = Some(schemas::yaml_settings(source));
            setup.pulls_settings = true;
            setup.associates_schemas = true;
            setup
                .notifications
                .push(("yaml/registerCustomSchemaRequest", json!({})));
            if closed {
                setup.notifications.push(("yaml/registerContentRequest", json!({})));
            }
        }
        SchemaClient::Json => {
            setup.settings = Some(schemas::json_settings());
            if closed {
                setup.options = Some(json!({"handledSchemaProtocols": ["file"]}));
            }
        }
    }
    setup.bundled = closed && language.schemas != SchemaClient::None;
    setup
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(compiler: Option<&str>) -> ServerCommand {
        ServerCommand {
            program: "server".into(),
            compiler: compiler.map(PathBuf::from),
        }
    }

    fn setup_of(language: &language::Language, command: &ServerCommand, source: SchemaSource) -> Setup {
        setup(language.backend().config(), command, Path::new("/project"), source)
    }

    #[test]
    fn typescript_is_told_where_its_compiler_is_and_other_languages_are_told_nothing() {
        let options = setup_of(
            &language::TYPESCRIPT,
            &command(Some("/p/typescript/lib/tsserver.js")),
            SchemaSource::Bundled,
        )
        .options;
        assert_eq!(
            options,
            Some(json!({"tsserver": {"path": "/p/typescript/lib/tsserver.js"}}))
        );
        assert_eq!(
            setup_of(&language::TYPESCRIPT, &command(None), SchemaSource::Bundled).options,
            None
        );
        let rust = setup_of(&language::RUST, &command(Some("/x")), SchemaSource::Online);
        assert_eq!((rust.options, rust.settings, rust.bundled), (None, None, false));
    }

    #[test]
    fn angular_looks_for_typescript_and_its_language_service_in_the_project_first() {
        let tsserver = "/project/node_modules/typescript/lib/tsserver.js";
        let args = setup_of(&language::ANGULAR, &command(Some(tsserver)), SchemaSource::Bundled).args;
        assert_eq!(args[0], "--stdio");
        assert_eq!(args[1], "--tsProbeLocations");
        assert!(args[2].starts_with("/project/node_modules"), "{args:?}");
        assert_eq!(args[3], "--ngProbeLocations");
        assert!(args[4].starts_with("/project/node_modules"), "{args:?}");
    }

    #[test]
    fn a_closed_network_has_the_servers_ask_vet_for_every_schema() {
        let yaml = setup_of(&language::YAML, &command(None), SchemaSource::Bundled);
        assert!(yaml.bundled);
        assert!(
            yaml.notifications
                .iter()
                .any(|(m, _)| *m == "yaml/registerContentRequest")
        );
        assert_eq!(yaml.settings.unwrap()["yaml"]["schemaStore"]["enable"], json!(false));

        let json_server = setup_of(&language::JSON, &command(None), SchemaSource::Bundled);
        assert!(json_server.bundled);
        assert_eq!(json_server.options, Some(json!({"handledSchemaProtocols": ["file"]})));
    }

    #[test]
    fn an_open_network_leaves_the_loading_to_the_servers() {
        let yaml = setup_of(&language::YAML, &command(None), SchemaSource::Online);
        assert!(!yaml.bundled);
        assert!(
            !yaml
                .notifications
                .iter()
                .any(|(m, _)| *m == "yaml/registerContentRequest")
        );
        assert_eq!(yaml.settings.unwrap()["yaml"]["schemaStore"]["enable"], json!(true));

        let json_server = setup_of(&language::JSON, &command(None), SchemaSource::Online);
        assert!(!json_server.bundled);
        assert_eq!(json_server.options, None);
    }

    #[test]
    fn languages_without_schemas_never_have_them_bundled() {
        for language in [&language::BASH, &language::DOCKERFILE, &language::SVELTE] {
            let setup = setup_of(language, &command(None), SchemaSource::Bundled);
            assert!(!setup.bundled && setup.settings.is_none(), "{}", language.id);
        }
    }
}
