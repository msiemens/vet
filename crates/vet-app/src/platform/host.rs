//! Which platform a host name belongs to.

/// The hosting platforms Vet talks to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlatformKind {
    GitHub,
    AzureDevOps,
}

impl PlatformKind {
    /// Azure DevOps lives at `dev.azure.com` and at `<organization>.visualstudio.com`;
    /// any other host is a GitHub Enterprise installation.
    pub fn of_host(host: &str) -> Self {
        if host == "dev.azure.com" || host.ends_with(".visualstudio.com") {
            Self::AzureDevOps
        } else {
            Self::GitHub
        }
    }
}

/// The host name in `input`, as typed or pasted: lower case, without a scheme, a path or
/// surrounding space. The error is fit to show.
pub fn normalize_host(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host = without_scheme
        .split('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.is_empty() || host.contains(|c: char| c.is_whitespace() || c == '@') {
        return Err(format!("\"{trimmed}\" is not a host name, e.g. github.com"));
    }
    Ok(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pasted_url_becomes_its_host() {
        assert_eq!(normalize_host(" https://GitHub.com/org/repo ").unwrap(), "github.com");
        assert_eq!(
            normalize_host("ghe.corp.example:8443").unwrap(),
            "ghe.corp.example:8443"
        );
    }

    #[test]
    fn something_that_is_not_a_host_is_refused() {
        for input in ["", "  ", "https:///x", "two words", "user@github.com"] {
            assert!(normalize_host(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn azure_devops_hosts_are_told_from_github_enterprise_hosts() {
        assert_eq!(PlatformKind::of_host("dev.azure.com"), PlatformKind::AzureDevOps);
        assert_eq!(
            PlatformKind::of_host("contoso.visualstudio.com"),
            PlatformKind::AzureDevOps
        );
        assert_eq!(PlatformKind::of_host("github.com"), PlatformKind::GitHub);
        assert_eq!(PlatformKind::of_host("git.corp.example"), PlatformKind::GitHub);
    }
}
