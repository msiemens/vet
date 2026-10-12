//! Reusing the token of a signed-in `gh` or `az`: the command that prints it, and reading
//! what it printed.

use std::path::Path;

use super::credentials::Token;
use super::host::PlatformKind;
use crate::ports::Command;

/// Azure DevOps' application id, the resource an Entra token for it is issued for.
const AZURE_DEVOPS_RESOURCE: &str = "499b84ac-1321-427f-aa17-267ca6975798";

/// A token a CLI printed, and when it stops being good (unix seconds) if it said.
pub struct CliToken {
    pub token: Token,
    pub expires_at: Option<i64>,
}

pub fn command(kind: PlatformKind, host: &str, cwd: &Path) -> Command {
    let (program, args): (&str, Vec<&str>) = match kind {
        PlatformKind::GitHub => ("gh", vec!["auth", "token", "--hostname", host]),
        PlatformKind::AzureDevOps => (
            "az",
            vec![
                "account",
                "get-access-token",
                "--resource",
                AZURE_DEVOPS_RESOURCE,
                "--output",
                "json",
                "--only-show-errors",
            ],
        ),
    };
    Command {
        program: program.to_owned(),
        args: args.into_iter().map(str::to_owned).collect(),
        cwd: cwd.to_owned(),
        env: Vec::new(),
    }
}

/// The command that lists the hosts `gh` is signed in to, GitHub Enterprise ones too.
pub fn hosts_command(cwd: &Path) -> Command {
    Command {
        program: "gh".to_owned(),
        args: ["auth", "status", "--active", "--json", "hosts"]
            .map(str::to_owned)
            .into(),
        cwd: cwd.to_owned(),
        env: Vec::new(),
    }
}

/// A host `gh` knows, and what it says is wrong with its sign-in, if anything.
#[derive(Debug, PartialEq, Eq)]
pub struct GhHost {
    /// In lower case.
    pub host: String,
    /// `gh`'s `error` for the host's account, else its `state` when that is not `success`.
    pub problem: Option<String>,
}

/// The hosts in what `hosts_command` printed; none if it is not that.
pub fn parse_hosts(stdout: &str) -> Vec<GhHost> {
    let Ok(answer) = serde_json::from_str::<serde_json::Value>(stdout) else {
        return Vec::new();
    };
    let Some(hosts) = answer.get("hosts").and_then(serde_json::Value::as_object) else {
        return Vec::new();
    };
    hosts
        .iter()
        .map(|(host, accounts)| GhHost {
            host: host.to_lowercase(),
            problem: accounts.as_array().into_iter().flatten().find_map(account_problem),
        })
        .collect()
}

fn account_problem(account: &serde_json::Value) -> Option<String> {
    let state = account.get("state").and_then(serde_json::Value::as_str)?;
    if state == "success" {
        return None;
    }
    let error = account
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|error| !error.is_empty());
    Some(error.map_or_else(|| format!("state {state}"), str::to_owned))
}

/// The token in what the command printed, `None` if there is none.
pub fn parse(kind: PlatformKind, stdout: &str) -> Option<CliToken> {
    match kind {
        PlatformKind::GitHub => Some(CliToken {
            token: Token::new(stdout)?,
            expires_at: None,
        }),
        PlatformKind::AzureDevOps => {
            let answer: serde_json::Value = serde_json::from_str(stdout).ok()?;
            Some(CliToken {
                token: Token::new(answer.get("accessToken")?.as_str()?)?,
                // Older versions of `az` print only a local time, which is not enough.
                expires_at: answer.get("expires_on").and_then(serde_json::Value::as_i64),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh_prints_the_bare_token() {
        let cli = parse(PlatformKind::GitHub, "gho_abc\n").unwrap();
        assert_eq!(cli.token.expose(), "gho_abc");
        assert_eq!(cli.expires_at, None);
        assert!(parse(PlatformKind::GitHub, "\n").is_none());
    }

    #[test]
    fn gh_lists_the_hosts_it_is_signed_in_to() {
        let out = r#"{"hosts":{"github.com":[{"active":true,"host":"github.com","login":"octocat","state":"success"}],
"GHE.corp.example":[{"active":true,"host":"GHE.corp.example","login":"ada","state":"success"}]}}"#;
        let mut hosts: Vec<_> = parse_hosts(out).into_iter().map(|h| h.host).collect();
        hosts.sort();
        assert_eq!(hosts, ["ghe.corp.example", "github.com"]);
        assert!(parse_hosts("You are not logged into any GitHub hosts.").is_empty());
    }

    #[test]
    fn a_host_gh_does_not_report_as_success_has_a_problem_in_gh_s_words() {
        let out = r#"{"hosts":{"github.com":[{"active":true,"state":"error","error":"HTTP 401: Bad credentials"}],
"ghe.corp.example":[{"active":true,"state":"timeout"}],
"ghe.fine.example":[{"active":true,"state":"success"}]}}"#;
        let problems: Vec<_> = parse_hosts(out).into_iter().map(|h| (h.host, h.problem)).collect();
        assert_eq!(
            problems,
            [
                ("ghe.corp.example".to_owned(), Some("state timeout".to_owned())),
                ("ghe.fine.example".to_owned(), None),
                ("github.com".to_owned(), Some("HTTP 401: Bad credentials".to_owned())),
            ]
        );
    }

    #[test]
    fn az_prints_json_with_the_expiry() {
        let out = r#"{"accessToken":"eyJ0","expiresOn":"2030-01-01 10:00:00.000000","expires_on":1893492000,"tokenType":"Bearer"}"#;
        let cli = parse(PlatformKind::AzureDevOps, out).unwrap();
        assert_eq!(cli.token.expose(), "eyJ0");
        assert_eq!(cli.expires_at, Some(1893492000));
        assert!(parse(PlatformKind::AzureDevOps, "ERROR: Please run 'az login'").is_none());
    }
}
