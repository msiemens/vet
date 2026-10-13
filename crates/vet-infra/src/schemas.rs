//! The JSON schemas the YAML and JSON language servers check files against (§7.5).
//!
//! A file can name a schema by address (`$schema`, a modeline), and the YAML server also reads
//! the schema store. Loading those is a request to an address the repository chose, so it is
//! allowed only where the analysis policy allows it ([`SchemaSource`]). Elsewhere Vet answers
//! every request for a schema itself, from a fixed set bundled with it, and refuses the rest.
//! The same files are what a server uses for the well-known file names in both cases; online,
//! the server loads them from the address they are published at.

use std::sync::OnceLock;

use serde_json::{Value, json};
use vet_core::content::Hash;
use vet_semantic::policy::AnalysisPolicy;

use crate::restore::record::Digest;

/// Where the schemas a server checks files against may come from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SchemaSource {
    /// Only the bundle: a request for any other address is refused.
    Bundled,
    /// Whatever address the file names, and the schema store, from the network.
    Online,
}

impl SchemaSource {
    pub fn of(policy: AnalysisPolicy) -> Self {
        if policy.allow_schema_downloads() {
            Self::Online
        } else {
            Self::Bundled
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::Online => "online",
        }
    }
}

/// A bundled schema: the addresses it is published at, and the file names it is for.
struct Schema {
    /// The first address is the one the file associations name.
    urls: &'static [&'static str],
    body: &'static str,
    /// Globs of the files a YAML server checks against it.
    yaml: &'static [&'static str],
    /// Globs of the files the JSON server checks against it.
    json: &'static [&'static str],
}

const SCHEMAS: &[Schema] = &[
    Schema {
        urls: &[
            "https://json.schemastore.org/github-workflow.json",
            "https://json.schemastore.org/github-workflow",
            "https://www.schemastore.org/github-workflow.json",
        ],
        body: include_str!("../schemas/github-workflow.schema.json"),
        yaml: &["**/.github/workflows/*.yml", "**/.github/workflows/*.yaml"],
        json: &[],
    },
    Schema {
        urls: &[
            "https://raw.githubusercontent.com/microsoft/azure-pipelines-vscode/main/service-schema.json",
            "https://raw.githubusercontent.com/microsoft/azure-pipelines-vscode/master/service-schema.json",
        ],
        body: include_str!("../schemas/azure-pipelines.schema.json"),
        yaml: &[
            "**/azure-pipelines.yml",
            "**/azure-pipelines.yaml",
            "**/azure-pipelines/*.yml",
            "**/azure-pipelines/*.yaml",
            "**/.azure-pipelines/*.yml",
            "**/.azure-pipelines/*.yaml",
        ],
        json: &[],
    },
    Schema {
        urls: &[
            "https://raw.githubusercontent.com/compose-spec/compose-spec/main/schema/compose-spec.json",
            "https://raw.githubusercontent.com/compose-spec/compose-spec/master/schema/compose-spec.json",
            "https://json.schemastore.org/docker-compose.json",
        ],
        body: include_str!("../schemas/compose-spec.schema.json"),
        yaml: &[
            "**/docker-compose.yml",
            "**/docker-compose.yaml",
            "**/docker-compose.*.yml",
            "**/docker-compose.*.yaml",
            "**/compose.yml",
            "**/compose.yaml",
            "**/compose.*.yml",
            "**/compose.*.yaml",
        ],
        json: &[],
    },
    Schema {
        urls: &[
            "https://json.schemastore.org/package.json",
            "https://json.schemastore.org/package",
            "https://www.schemastore.org/package.json",
        ],
        body: include_str!("../schemas/package-json.schema.json"),
        yaml: &[],
        json: &["**/package.json"],
    },
    Schema {
        urls: &[
            "https://json.schemastore.org/tsconfig.json",
            "https://json.schemastore.org/tsconfig",
            "https://www.schemastore.org/tsconfig.json",
        ],
        body: include_str!("../schemas/tsconfig.schema.json"),
        yaml: &[],
        json: &["**/tsconfig.json", "**/tsconfig.*.json"],
    },
];

/// The Kubernetes schema: `all.json` lists the kinds and refers to `_definitions.json`, which
/// holds them. The YAML server loads both for a version of Kubernetes it names itself.
const KUBERNETES_ALL: &str = include_str!("../schemas/kubernetes-all.schema.json");
const KUBERNETES_DEFINITIONS: &str = include_str!("../schemas/kubernetes-definitions.schema.json");
const KUBERNETES_VERSION: &str = "1.34.0";
const KUBERNETES_BASE: &str = "https://raw.githubusercontent.com/yannh/kubernetes-json-schema/master/";
/// Files in these folders are taken for Kubernetes manifests. The server puts `**/` before a glob
/// and that skips every directory whose name starts with a dot, such as the ones above a project
/// kept under `~/.local`; `*(*/|.*/)` takes any directories, dotted or not.
const KUBERNETES_GLOBS: &[&str] = &[
    "*(*/|.*/)k8s/**/*.yml",
    "*(*/|.*/)k8s/**/*.yaml",
    "*(*/|.*/)kubernetes/**/*.yml",
    "*(*/|.*/)kubernetes/**/*.yaml",
    "*(*/|.*/)manifests/**/*.yml",
    "*(*/|.*/)manifests/**/*.yaml",
];

/// The Kubernetes file at `url`, whichever version of the schema it is for.
fn kubernetes(url: &str) -> Option<&'static str> {
    let rest = url.strip_prefix(KUBERNETES_BASE)?;
    let (version, file) = rest.split_once('/')?;
    let numbers = version.strip_prefix('v')?.strip_suffix("-standalone-strict")?;
    let parts: Vec<_> = numbers.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    match file {
        "all.json" => Some(KUBERNETES_ALL),
        "_definitions.json" => Some(KUBERNETES_DEFINITIONS),
        _ => None,
    }
}

/// The bundled schema published at `url`; a fragment names a part of it, which the server picks.
fn bundled(url: &str) -> Option<&'static str> {
    let url = url.split_once('#').map_or(url, |(address, _)| address);
    SCHEMAS
        .iter()
        .find(|s| s.urls.contains(&url))
        .map(|s| s.body)
        .or_else(|| kubernetes(url))
}

/// The answer to a server's request for the schema at `url` when the network is closed: the
/// bundled schema, else the reason there is none.
pub fn serve(url: &str) -> Result<String, String> {
    bundled(url).map(str::to_owned).ok_or_else(|| {
        format!("{url} is not one of the schemas bundled with Vet, and downloads are off until the project is trusted")
    })
}

/// A hash of everything that is bundled and how it is associated with files: the schemas a
/// server is given when the network is closed.
pub fn fingerprint() -> Hash {
    static FINGERPRINT: OnceLock<Hash> = OnceLock::new();
    *FINGERPRINT.get_or_init(|| {
        let mut digest = Digest::default();
        digest.field("bundled schemas 1");
        for schema in SCHEMAS {
            digest.field(&schema.urls.join(" ")).field(schema.body);
            digest.field(&schema.yaml.join(" ")).field(&schema.json.join(" "));
        }
        digest
            .field(KUBERNETES_VERSION)
            .field(KUBERNETES_ALL)
            .field(KUBERNETES_DEFINITIONS);
        digest.field(&KUBERNETES_GLOBS.join(" "));
        digest.finish()
    })
}

/// Whether `path` (with `/` between its names) is matched by `pattern`: names are matched one by
/// one, `*` stands for any characters of one name and a name of `**` for any number of names.
/// Unlike the servers' own globs, a name that starts with a dot is matched like any other: the
/// directories above a project are not ours to choose.
fn glob(pattern: &str, path: &str) -> bool {
    fn name(pattern: &[u8], text: &[u8]) -> bool {
        match pattern.split_first() {
            None => text.is_empty(),
            Some((b'*', rest)) => (0..=text.len()).any(|skip| name(rest, &text[skip..])),
            Some((c, rest)) => text.first() == Some(c) && name(rest, &text[1..]),
        }
    }
    fn names(pattern: &[&str], path: &[&str]) -> bool {
        match pattern.split_first() {
            None => path.is_empty(),
            Some((&"**", rest)) => (0..=path.len()).any(|skip| names(rest, &path[skip..])),
            Some((first, rest)) => {
                path.first().is_some_and(|n| name(first.as_bytes(), n.as_bytes())) && names(rest, &path[1..])
            }
        }
    }
    let (pattern, path): (Vec<_>, Vec<_>) = (pattern.split('/').collect(), path.split('/').collect());
    names(&pattern, &path)
}

/// The addresses of the schemas the YAML file at `path` is checked against by file name.
pub fn yaml_schemas(path: &str) -> Vec<&'static str> {
    let path = path.replace('\\', "/");
    SCHEMAS
        .iter()
        .filter(|s| s.yaml.iter().any(|g| glob(g, &path)))
        .map(|s| s.urls[0])
        .collect()
}

/// The settings of the YAML server (`yaml-language-server`): the schema store only online, and
/// Kubernetes manifests in the folders `KUBERNETES_GLOBS` name. The server picks a manifest's
/// schema from the Kubernetes one by its kind only for files it matched itself, and only with
/// the CRD catalog on; offline, a custom resource's schema is a request Vet refuses. Which other
/// file has which schema is answered by [`yaml_schemas`]; the Kubernetes folders are the one set
/// the server matches.
pub fn yaml_settings(source: SchemaSource) -> Value {
    let online = source == SchemaSource::Online;
    json!({
        "yaml": {
            "schemas": {"kubernetes": KUBERNETES_GLOBS},
            "schemaStore": {"enable": online},
            "kubernetesCRDStore": {"enable": true},
            "kubernetesVersion": KUBERNETES_VERSION,
            "validate": true,
            "format": {"enable": false},
        },
    })
}

/// The settings of the JSON server (`vscode-json-language-server`).
pub fn json_settings() -> Value {
    let schemas: Vec<Value> = SCHEMAS
        .iter()
        .filter(|s| !s.json.is_empty())
        .map(|s| json!({"fileMatch": s.json, "url": s.urls[0]}))
        .collect();
    json!({"json": {"schemas": schemas, "validate": {"enable": true}}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_address_is_served_and_the_rest_is_refused() {
        for schema in SCHEMAS {
            for url in schema.urls {
                assert_eq!(serve(url).as_deref(), Ok(schema.body), "{url}");
            }
        }
        let refused = serve("https://example.org/own.schema.json").unwrap_err();
        assert!(
            refused.contains("example.org") && refused.contains("trusted"),
            "{refused}"
        );
        assert!(serve("http://json.schemastore.org/package.json").is_err());
        assert!(serve("https://json.schemastore.org/package.json?x=1").is_err());
    }

    #[test]
    fn kubernetes_is_served_for_any_version_of_its_two_files() {
        let at = |version: &str, file: &str| format!("{KUBERNETES_BASE}{version}-standalone-strict/{file}");
        assert_eq!(serve(&at("v1.34.0", "all.json")).as_deref(), Ok(KUBERNETES_ALL));
        assert_eq!(
            serve(&at("v1.19.12", "_definitions.json")).as_deref(),
            Ok(KUBERNETES_DEFINITIONS)
        );
        assert_eq!(
            serve(&format!(
                "{}#/definitions/io.k8s.api.apps.v1.Deployment",
                at("v1.34.0", "_definitions.json")
            ))
            .as_deref(),
            Ok(KUBERNETES_DEFINITIONS),
            "a fragment names a part of the schema"
        );
        assert!(serve(&at("v1.34", "all.json")).is_err());
        assert!(serve(&at("v1.34.0", "deployment.json")).is_err());
        assert!(serve(&at("master", "all.json")).is_err());
        assert!(serve(&format!("{KUBERNETES_BASE}v1.34.0-standalone/all.json")).is_err());
    }

    #[test]
    fn the_schema_store_is_enabled_only_for_the_online_source() {
        let store = |source| yaml_settings(source)["yaml"]["schemaStore"]["enable"].clone();
        assert_eq!(store(SchemaSource::Bundled), json!(false));
        assert_eq!(store(SchemaSource::Online), json!(true));
    }

    #[test]
    fn the_source_follows_the_policys_permission_to_download() {
        assert_eq!(SchemaSource::of(AnalysisPolicy::LIMITED), SchemaSource::Bundled);
        assert_eq!(SchemaSource::of(AnalysisPolicy::SERVERS), SchemaSource::Bundled);
        assert_eq!(SchemaSource::of(AnalysisPolicy::EXECUTING), SchemaSource::Online);
    }

    #[test]
    fn yaml_files_get_their_schemas_by_name_whatever_the_directories_above_them_are_called() {
        let github = SCHEMAS[0].urls[0];
        let compose = SCHEMAS[2].urls[0];
        let azure = SCHEMAS[1].urls[0];
        assert_eq!(yaml_schemas("/home/u/.work/app/.github/workflows/ci.yml"), [github]);
        assert_eq!(yaml_schemas("/p/docker-compose.override.yaml"), [compose]);
        assert_eq!(yaml_schemas("/p/compose.yml"), [compose]);
        assert_eq!(yaml_schemas("C:\\p\\azure-pipelines.yml"), [azure]);
        assert!(yaml_schemas("/p/src/config.yaml").is_empty());
        assert!(
            yaml_schemas("/p/deploy/k8s/base/app.yaml").is_empty(),
            "the server picks Kubernetes files"
        );
        assert!(yaml_schemas("/p/.github/workflows/nested/ci.yml").is_empty());
        assert!(yaml_schemas("/p/.github/ci.yml").is_empty());
    }

    #[test]
    fn the_json_associations_name_only_addresses_that_are_served() {
        for entry in json_settings()["json"]["schemas"].as_array().unwrap() {
            assert!(bundled(entry["url"].as_str().unwrap()).is_some(), "{entry}");
        }
    }
}
