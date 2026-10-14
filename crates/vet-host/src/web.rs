//! Which addresses the shell opens in the user's browser.

/// `url` if it is a web address that is safe to hand to the system's opener as one argument.
pub fn web_address(url: &str) -> Result<&str, String> {
    let web = url.starts_with("https://") || url.starts_with("http://");
    if web && !url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Ok(url)
    } else {
        Err(format!("{url:?} is not a web address"))
    }
}

#[cfg(test)]
mod tests {
    use super::web_address;

    #[test]
    fn only_web_addresses_without_spaces_are_opened() {
        assert!(web_address("https://github.com/acme/app/commit/abc/checks").is_ok());
        assert!(web_address("http://localhost:3000/x?a=1&b=2").is_ok());
        for refused in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ssh://host",
            "httpx://host",
            "github.com",
            "https://a b",
            "https://a\nb",
            "",
        ] {
            assert!(web_address(refused).is_err(), "{refused:?}");
        }
    }
}
