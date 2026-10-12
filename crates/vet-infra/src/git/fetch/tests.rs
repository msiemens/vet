use super::{https_host, stays_in_remotes};

fn url(s: &str) -> gix::Url {
    gix::url::parse(s).unwrap()
}

fn spec(s: &str) -> gix::refspec::RefSpec {
    gix::refspec::parse(s.into(), gix::refspec::parse::Operation::Fetch)
        .unwrap()
        .to_owned()
}

#[test]
fn an_https_remote_has_its_host_in_lower_case_with_its_port_and_without_credentials() {
    assert_eq!(
        https_host(&url("https://GitHub.com/o/r.git")).as_deref(),
        Some("github.com")
    );
    assert_eq!(
        https_host(&url("https://user:secret@Git.Example.com:8443/o/r")).as_deref(),
        Some("git.example.com:8443")
    );
}

#[test]
fn a_remote_that_is_not_https_has_no_host_to_offer_a_token_to() {
    for remote in [
        "http://github.com/o/r",
        "ssh://git@github.com/o/r",
        "git@github.com:o/r.git",
        "git://github.com/o/r",
        "/srv/repo.git",
    ] {
        assert_eq!(https_host(&url(remote)), None, "{remote}");
    }
}

#[test]
fn only_refspecs_that_write_below_refs_remotes_are_followed() {
    for followed in [
        "+refs/heads/*:refs/remotes/origin/*",
        "refs/heads/main:refs/remotes/origin/main",
        "^refs/heads/wip",
    ] {
        assert!(stays_in_remotes(&spec(followed)), "{followed}");
    }
    for refused in [
        "+refs/heads/*:refs/heads/*",
        "refs/tags/*:refs/tags/*",
        "refs/heads/main",
        "refs/heads/*:refs/remotes-other/*",
        "refs/pull/*/head:refs/vet/pr/*",
    ] {
        assert!(!stays_in_remotes(&spec(refused)), "{refused}");
    }
}
