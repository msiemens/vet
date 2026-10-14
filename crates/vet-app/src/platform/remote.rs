//! Which repository on which host a remote URL names.

use std::borrow::Cow;

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};

use super::host::{PlatformKind, azure_devops_account};

/// What is escaped in a name of an Azure DevOps path: whatever a URL path cannot carry as it
/// is, the escape itself, and the separators.
const ESCAPED: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'\\');

/// A repository on a hosting platform: `github.com` and `owner/repo`, or
/// `dev.azure.com` and `organization/project/repo`. The same for every URL form that
/// reaches the repository. An Azure DevOps name is kept as a URL path carries it: `My%20Shop`
/// for the project `My Shop`, however the remote spells it.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct RemoteKey {
    pub host: String,
    pub path: String,
}

impl RemoteKey {
    /// The platform Vet knows this host to be by name, if any. A GitHub Enterprise installation is
    /// not one: it is GitHub only through the viewer's account (see [`Platforms`]).
    pub fn kind(&self) -> Option<PlatformKind> {
        PlatformKind::of_host(&self.host)
    }

    /// The host of the account that reaches the repository: the host itself, except for
    /// Azure DevOps, whose accounts are of one organization (`dev.azure.com/<organization>`,
    /// whichever of its hosts the remote names).
    pub fn account(&self) -> String {
        match (self.kind(), self.segments().next()) {
            (Some(PlatformKind::AzureDevOps), Some(organization)) => azure_devops_account(organization),
            _ => self.host.clone(),
        }
    }

    /// The segments of the path: owner and repository for GitHub, organization, project
    /// and repository for Azure DevOps.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.path.split('/')
    }

    /// The segments as the names they are: `My Shop` for `My%20Shop`.
    pub fn names(&self) -> impl Iterator<Item = Cow<'_, str>> {
        self.segments().map(|s| percent_decode_str(s).decode_utf8_lossy())
    }
}

impl std::fmt::Display for RemoteKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.host, self.path)
    }
}

/// How a URL reaches the host. Only a web URL's port is the web (and API) port.
enum Transport {
    Web { default_port: &'static str },
    Ssh,
}

/// The repository `url` (as `git remote get-url` prints it) names, if it is one on a
/// platform Vet speaks to. Local paths and URLs of other shapes are none.
pub fn parse_remote(url: &str) -> Option<RemoteKey> {
    let url = url.trim();
    let (transport, authority, path) = match url.split_once("://") {
        Some((scheme, rest)) => {
            let transport = match scheme.to_ascii_lowercase().as_str() {
                "https" => Transport::Web { default_port: "443" },
                "http" => Transport::Web { default_port: "80" },
                "ssh" | "git" | "ssh+git" | "git+ssh" => Transport::Ssh,
                _ => return None,
            };
            let (authority, path) = rest.split_once('/')?;
            (transport, authority, path)
        }
        // scp-like: `[user@]host:path`.
        None => {
            let (authority, path) = url.split_once(':')?;
            if authority.contains('/') {
                return None;
            }
            (Transport::Ssh, authority, path)
        }
    };
    let (host, port) = split_host(authority.rsplit('@').next()?)?;
    // `C:\repo` is a drive.
    if host.len() == 1 {
        return None;
    }
    let host = match (transport, port) {
        (Transport::Web { default_port }, Some(port)) if port != default_port => format!("{host}:{port}"),
        _ => host,
    };
    let path = path.split(['?', '#']).next()?.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let segments: Vec<&str> = path.split('/').collect();
    if segments.iter().any(|s| s.is_empty() || *s == "." || *s == "..") {
        return None;
    }
    if let Some(key) = azure_devops_ssh(&host, &segments) {
        return Some(key);
    }
    match PlatformKind::of_host(&host) {
        Some(PlatformKind::AzureDevOps) => azure_devops(&host, &segments),
        // github.com and any host whose shape is GitHub's. Whether the host is a platform Vet talks
        // to is the registry's to say; this only tells the shape of the path.
        _ => github(&host, &segments),
    }
}

fn split_host(authority: &str) -> Option<(String, Option<String>)> {
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            (host, Some(port.to_owned()))
        }
        Some(_) => return None,
        None => (authority, None),
    };
    (!host.is_empty() && !host.contains(char::is_whitespace)).then(|| (host.to_ascii_lowercase(), port))
}

fn github(host: &str, segments: &[&str]) -> Option<RemoteKey> {
    // GitHub's SSH endpoint on the HTTPS port.
    let host = if host == "ssh.github.com" { "github.com" } else { host };
    let [owner, repo] = segments else { return None };
    Some(RemoteKey {
        host: host.to_owned(),
        path: format!("{owner}/{repo}"),
    })
}

/// `https://dev.azure.com/org/project/_git/repo` and
/// `https://org.visualstudio.com/[DefaultCollection/]project/_git/repo`, either also with
/// `_optimized/` or `_full/` before the repository.
fn azure_devops(host: &str, segments: &[&str]) -> Option<RemoteKey> {
    let (org, rest) = match segments {
        [org, rest @ ..] if host == "dev.azure.com" => (*org, rest),
        rest => {
            let rest = rest.strip_prefix(&["DefaultCollection"]).unwrap_or(rest);
            (host.strip_suffix(".visualstudio.com")?, rest)
        }
    };
    let (project, repo) = match rest {
        [project, "_git", repo] | [project, "_git", "_optimized" | "_full", repo] => (*project, *repo),
        // A repository named like its project may leave the project out.
        ["_git", repo] | ["_git", "_optimized" | "_full", repo] => (*repo, *repo),
        _ => return None,
    };
    Some(RemoteKey {
        host: host.to_owned(),
        path: azure_devops_path(org, project, repo)?,
    })
}

/// The path of an Azure DevOps repository, its names escaped as `ESCAPED` says whether or
/// not the remote escaped them.
fn azure_devops_path(org: &str, project: &str, repo: &str) -> Option<String> {
    let escaped = |name: &str| {
        let name = percent_decode_str(name).decode_utf8().ok()?;
        Some(utf8_percent_encode(&name, ESCAPED).to_string())
    };
    Some(format!("{}/{}/{}", escaped(org)?, escaped(project)?, escaped(repo)?))
}

/// `git@ssh.dev.azure.com:v3/org/project/repo` and
/// `org@vs-ssh.visualstudio.com:v3/org/project/repo`.
fn azure_devops_ssh(host: &str, segments: &[&str]) -> Option<RemoteKey> {
    let &["v3", org, project, repo] = segments else {
        return None;
    };
    let host = if host == "ssh.dev.azure.com" {
        "dev.azure.com".to_owned()
    } else if host.ends_with("vs-ssh.visualstudio.com") {
        format!("{org}.visualstudio.com")
    } else {
        return None;
    };
    Some(RemoteKey {
        host,
        path: azure_devops_path(org, project, repo)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(host: &str, path: &str) -> Option<RemoteKey> {
        Some(RemoteKey {
            host: host.to_owned(),
            path: path.to_owned(),
        })
    }

    #[test]
    fn every_url_form_of_a_github_repository_is_the_same_key() {
        for url in [
            "https://github.com/rust-lang/rust",
            "https://github.com/rust-lang/rust.git",
            "https://github.com/rust-lang/rust/",
            "https://user:token@github.com/rust-lang/rust.git",
            "HTTPS://GitHub.com/rust-lang/rust",
            "git@github.com:rust-lang/rust.git",
            "github.com:rust-lang/rust",
            "ssh://git@github.com/rust-lang/rust.git",
            "ssh://git@github.com:22/rust-lang/rust.git",
            "ssh://git@ssh.github.com:443/rust-lang/rust.git",
            "git://github.com/rust-lang/rust.git",
        ] {
            assert_eq!(parse_remote(url), key("github.com", "rust-lang/rust"), "{url}");
        }
    }

    #[test]
    fn an_enterprise_host_keeps_its_name_and_its_web_port() {
        assert_eq!(
            parse_remote("https://ghe.corp.example/team/service.git"),
            key("ghe.corp.example", "team/service")
        );
        assert_eq!(
            parse_remote("https://ghe.corp.example:8443/team/service"),
            key("ghe.corp.example:8443", "team/service")
        );
        assert_eq!(
            parse_remote("git@ghe.corp.example:team/service.git"),
            key("ghe.corp.example", "team/service")
        );
        assert_eq!(
            parse_remote("ssh://git@ghe.corp.example:2222/team/service.git"),
            key("ghe.corp.example", "team/service"),
            "the SSH port is not the web port"
        );
        assert_eq!(
            parse_remote("https://github.com:443/a/b"),
            key("github.com", "a/b"),
            "the default port is no part of the host"
        );
    }

    #[test]
    fn azure_devops_urls_name_organization_project_and_repository() {
        for url in [
            "https://dev.azure.com/contoso/Shop/_git/api",
            "https://contoso@dev.azure.com/contoso/Shop/_git/api",
            "git@ssh.dev.azure.com:v3/contoso/Shop/api",
            "ssh://git@ssh.dev.azure.com/v3/contoso/Shop/api",
        ] {
            assert_eq!(parse_remote(url), key("dev.azure.com", "contoso/Shop/api"), "{url}");
            assert_eq!(
                parse_remote(url).unwrap().kind(),
                Some(PlatformKind::AzureDevOps),
                "{url}"
            );
        }
        assert_eq!(
            parse_remote("https://dev.azure.com/contoso/_git/api"),
            key("dev.azure.com", "contoso/api/api")
        );
        assert_eq!(
            parse_remote("https://contoso.visualstudio.com/DefaultCollection/Shop/_git/api"),
            key("contoso.visualstudio.com", "contoso/Shop/api")
        );
        assert_eq!(
            parse_remote("https://contoso.visualstudio.com/Shop/_git/api"),
            key("contoso.visualstudio.com", "contoso/Shop/api")
        );
        assert_eq!(
            parse_remote("contoso@vs-ssh.visualstudio.com:v3/contoso/Shop/api"),
            key("contoso.visualstudio.com", "contoso/Shop/api")
        );
    }

    #[test]
    fn azure_devops_names_with_spaces_and_other_letters_are_the_same_key_however_the_remote_escapes_them() {
        for url in [
            "https://dev.azure.com/vivavis/My%20Shop/_git/Back%20End",
            "https://vivavis@dev.azure.com/vivavis/My%20Shop/_git/Back%20End",
            "https://dev.azure.com/vivavis/My Shop/_git/Back End",
            "git@ssh.dev.azure.com:v3/vivavis/My%20Shop/Back%20End",
            "ssh://git@ssh.dev.azure.com/v3/vivavis/My%20Shop/Back%20End",
            "https://dev.azure.com/vivavis/My%20Shop/_git/_optimized/Back%20End",
            "https://dev.azure.com/vivavis/My%20Shop/_git/_full/Back%20End",
        ] {
            assert_eq!(
                parse_remote(url),
                key("dev.azure.com", "vivavis/My%20Shop/Back%20End"),
                "{url}"
            );
        }
        for url in [
            "https://vivavis.visualstudio.com/DefaultCollection/My%20Shop/_git/Back%20End",
            "https://vivavis.visualstudio.com/My Shop/_git/Back End",
            "vivavis@vs-ssh.visualstudio.com:v3/vivavis/My%20Shop/Back%20End",
        ] {
            assert_eq!(
                parse_remote(url),
                key("vivavis.visualstudio.com", "vivavis/My%20Shop/Back%20End"),
                "{url}"
            );
        }
        let umlaut = parse_remote("https://dev.azure.com/vivavis/Gesch%C3%A4ft/_git/Z%C3%A4hler").unwrap();
        assert_eq!(
            parse_remote("https://dev.azure.com/vivavis/Geschäft/_git/Zähler"),
            Some(umlaut.clone())
        );
        assert_eq!(umlaut.names().collect::<Vec<_>>(), ["vivavis", "Geschäft", "Zähler"]);
        assert_eq!(umlaut.account(), "dev.azure.com/vivavis");
        // An escaped escape is a percent sign in the name, not an escape.
        assert_eq!(
            parse_remote("https://dev.azure.com/vivavis/Shop/_git/100%25"),
            key("dev.azure.com", "vivavis/Shop/100%25")
        );
        assert_eq!(
            parse_remote("https://dev.azure.com/vivavis/_git/My%20Shop"),
            key("dev.azure.com", "vivavis/My%20Shop/My%20Shop")
        );
    }

    #[test]
    fn what_is_not_a_repository_on_a_platform_is_none() {
        for url in [
            "",
            "/home/me/repo",
            "../repo",
            "C:\\work\\repo",
            "file:///home/me/repo.git",
            "https://github.com/rust-lang",
            "https://github.com/rust-lang/rust/pulls",
            "https://dev.azure.com/contoso",
            "ftp://github.com/a/b",
            "git@github.com:a//b",
        ] {
            assert_eq!(parse_remote(url), None, "{url:?}");
        }
    }

    #[test]
    fn an_azure_devops_remote_is_reached_by_the_account_of_its_organization() {
        for url in [
            "https://dev.azure.com/Contoso/Shop/_git/api",
            "https://contoso.visualstudio.com/Shop/_git/api",
            "git@ssh.dev.azure.com:v3/contoso/Shop/api",
        ] {
            assert_eq!(parse_remote(url).unwrap().account(), "dev.azure.com/contoso", "{url}");
        }
        let other = parse_remote("https://dev.azure.com/fabrikam/Shop/_git/api").unwrap();
        assert_eq!(other.account(), "dev.azure.com/fabrikam");
        let github = parse_remote("https://ghe.corp.example/team/service").unwrap();
        assert_eq!(github.account(), "ghe.corp.example");
    }
}
