//! Which platform a host name belongs to.

/// The hosting platforms Vet talks to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlatformKind {
    GitHub,
    AzureDevOps,
}

impl PlatformKind {
    /// The host's platform, for the hosts Vet knows by name: `github.com` (also `ssh.github.com`)
    /// and the Azure DevOps hosts (`dev.azure.com`, with an account's host there being
    /// `dev.azure.com/<organization>`, and `<organization>.visualstudio.com`). Any other host is
    /// none here: a GitHub Enterprise installation is GitHub only because the viewer has an account
    /// for it, which [`Platforms`] knows.
    pub fn of_host(host: &str) -> Option<Self> {
        if host == AZURE_DEVOPS || host.starts_with("dev.azure.com/") || host.ends_with(".visualstudio.com") {
            Some(Self::AzureDevOps)
        } else if host == GITHUB_COM || host == SSH_GITHUB_COM {
            Some(Self::GitHub)
        } else {
            None
        }
    }

    /// The platform of an account the viewer adds for `host`, or of a remote a credential was found
    /// for: an Azure DevOps host by name, and otherwise GitHub, the only other platform Vet talks
    /// to (a GitHub Enterprise installation). Unlike [`PlatformKind::of_host`], this is about a host
    /// the viewer signed in to, so an unknown host is taken for GitHub rather than left open.
    pub fn of_account(host: &str) -> Self {
        match Self::of_host(host) {
            Some(PlatformKind::AzureDevOps) => PlatformKind::AzureDevOps,
            _ => PlatformKind::GitHub,
        }
    }
}

const AZURE_DEVOPS: &str = "dev.azure.com";
const GITHUB_COM: &str = "github.com";
const SSH_GITHUB_COM: &str = "ssh.github.com";

/// The platforms Vet talks to, by host: the hosts it knows by name and the hosts the viewer has an
/// account for (a GitHub Enterprise installation). A host that is none of these is unknown, and a
/// caller says so instead of taking it for GitHub.
#[derive(Clone, Debug, Default)]
pub struct Platforms {
    signed_in: Vec<String>,
}

impl Platforms {
    pub fn new() -> Self {
        Self::default()
    }

    /// A host the viewer has an account for: an installation Vet does not know by name is GitHub.
    pub fn with_account(mut self, host: &str) -> Self {
        self.signed_in.push(host.to_ascii_lowercase());
        self
    }

    /// Which platform `host` is, if any: a host Vet knows by name, or one the viewer has an account
    /// for (GitHub Enterprise). An unknown host is none.
    pub fn of(&self, host: &str) -> Option<PlatformKind> {
        PlatformKind::of_host(host).or_else(|| {
            let host = host.to_ascii_lowercase();
            self.signed_in.contains(&host).then_some(PlatformKind::GitHub)
        })
    }
}

/// The host of the account of an Azure DevOps organization.
pub(super) fn azure_devops_account(organization: &str) -> String {
    format!("{AZURE_DEVOPS}/{}", organization.to_ascii_lowercase())
}

/// The host of an account, as typed or pasted: lower case, without a scheme or surrounding
/// space. A GitHub host loses its path; an Azure DevOps account is of one organization, so
/// `dev.azure.com/<organization>` (also from a pasted repository address or an
/// `<organization>.visualstudio.com`). The error is fit to show.
pub fn normalize_host(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let mut segments = without_scheme.split('/');
    let host = segments.next().unwrap_or_default().to_ascii_lowercase();
    if host.is_empty() || host.contains(|c: char| c.is_whitespace() || c == '@') {
        return Err(format!("\"{trimmed}\" is not a host name, e.g. github.com"));
    }
    let organization = if host == AZURE_DEVOPS {
        match segments.next().filter(|s| !s.is_empty()) {
            Some(organization) => organization,
            None => {
                return Err(
                    "Azure DevOps accounts are of one organization: enter dev.azure.com/<organization>, \
                            e.g. dev.azure.com/contoso"
                        .to_owned(),
                );
            }
        }
    } else if let Some(organization) = host.strip_suffix(".visualstudio.com") {
        organization
    } else {
        return Ok(host);
    };
    if organization.is_empty()
        || !organization
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!(
            "\"{organization}\" is not an Azure DevOps organization name, e.g. dev.azure.com/contoso"
        ));
    }
    Ok(azure_devops_account(organization))
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
    fn an_azure_devops_account_is_of_one_organization() {
        for input in [
            "dev.azure.com/Contoso",
            "https://dev.azure.com/contoso/Shop/_git/api",
            "contoso.visualstudio.com",
            "https://contoso.visualstudio.com/DefaultCollection/Shop",
        ] {
            assert_eq!(normalize_host(input).unwrap(), "dev.azure.com/contoso", "{input}");
        }
        for input in [
            "dev.azure.com",
            "https://dev.azure.com/",
            "dev.azure.com/bad name",
            ".visualstudio.com",
        ] {
            assert!(normalize_host(input).is_err(), "{input:?}");
        }
        let why = normalize_host("dev.azure.com").unwrap_err();
        assert!(why.contains("dev.azure.com/<organization>"), "{why}");
    }

    #[test]
    fn something_that_is_not_a_host_is_refused() {
        for input in ["", "  ", "https:///x", "two words", "user@github.com"] {
            assert!(normalize_host(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn azure_devops_hosts_are_told_from_github_and_an_unknown_host_is_none() {
        assert_eq!(
            PlatformKind::of_host("dev.azure.com/contoso"),
            Some(PlatformKind::AzureDevOps)
        );
        assert_eq!(
            PlatformKind::of_host("contoso.visualstudio.com"),
            Some(PlatformKind::AzureDevOps)
        );
        assert_eq!(PlatformKind::of_host("github.com"), Some(PlatformKind::GitHub));
        assert_eq!(PlatformKind::of_host("ssh.github.com"), Some(PlatformKind::GitHub));
        assert_eq!(
            PlatformKind::of_host("ghe.corp.example"),
            None,
            "an unknown host is not GitHub by name"
        );
    }

    #[test]
    fn only_the_known_hosts_and_the_signed_in_ones_are_a_platform() {
        let known = Platforms::new();
        assert_eq!(known.of("github.com"), Some(PlatformKind::GitHub));
        assert_eq!(known.of("dev.azure.com/contoso"), Some(PlatformKind::AzureDevOps));
        assert_eq!(
            known.of("gitlab.com"),
            None,
            "an unknown host is refused, never taken for GitHub"
        );

        let signed_in = Platforms::new().with_account("ghe.corp.example");
        assert_eq!(signed_in.of("ghe.corp.example"), Some(PlatformKind::GitHub));
        assert_eq!(signed_in.of("GHE.Corp.Example"), Some(PlatformKind::GitHub));
        assert_eq!(signed_in.of("gitlab.com"), None);
    }
}
