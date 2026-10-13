//! Which platform a host name belongs to.

/// The hosting platforms Vet talks to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlatformKind {
    GitHub,
    AzureDevOps,
}

impl PlatformKind {
    /// Azure DevOps lives at `dev.azure.com` (an account's host there is
    /// `dev.azure.com/<organization>`) and at `<organization>.visualstudio.com`; any other
    /// host is a GitHub Enterprise installation.
    pub fn of_host(host: &str) -> Self {
        if host == AZURE_DEVOPS || host.starts_with("dev.azure.com/") || host.ends_with(".visualstudio.com") {
            Self::AzureDevOps
        } else {
            Self::GitHub
        }
    }
}

const AZURE_DEVOPS: &str = "dev.azure.com";

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
    fn azure_devops_hosts_are_told_from_github_enterprise_hosts() {
        assert_eq!(
            PlatformKind::of_host("dev.azure.com/contoso"),
            PlatformKind::AzureDevOps
        );
        assert_eq!(
            PlatformKind::of_host("contoso.visualstudio.com"),
            PlatformKind::AzureDevOps
        );
        assert_eq!(PlatformKind::of_host("github.com"), PlatformKind::GitHub);
        assert_eq!(PlatformKind::of_host("git.corp.example"), PlatformKind::GitHub);
    }
}
