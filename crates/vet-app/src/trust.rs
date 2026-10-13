//! Trust (§7.5). Computes the `AnalysisPolicy` handed to the pool, so
//! `vet-semantic` never looks at provenance.

use vet_semantic::policy::AnalysisPolicy;

use crate::changeset::CommitId;

/// What decides how far analysis of a changeset may execute project code.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Trust {
    /// The project (the repository) is marked trusted.
    pub project: bool,
    pub origin: Origin,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Origin {
    /// The working tree or a branch of the repository.
    Local,
    PullRequest(PullRequestTrust),
}

/// What is stored of a pull request's trust.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequestTrust {
    /// The head is in a repository other than the one the pull request is opened against.
    pub fork: bool,
    /// The head commit the viewer trusted the pull request at: trust is for exactly one
    /// commit, so any new push voids it.
    pub trusted_head: Option<CommitId>,
}

impl PullRequestTrust {
    /// What is assumed of a pull request whose platform data was never stored.
    pub fn unknown() -> Self {
        Self {
            fork: true,
            trusted_head: None,
        }
    }
}

impl Trust {
    /// What analysis of the tree built on `commit` may execute.
    ///
    /// A trusted project's own code executes everywhere; a pull request from a fork needs
    /// its own trust even there. In a project that is not trusted the working tree and
    /// branches keep their servers (they are the reviewer's own code) but restores stay
    /// fetch-only, and a pull request is limited until trusted.
    pub fn policy(&self, commit: &CommitId) -> AnalysisPolicy {
        match &self.origin {
            Origin::Local if self.project => AnalysisPolicy::EXECUTING,
            Origin::Local => AnalysisPolicy::SERVERS,
            Origin::PullRequest(PullRequestTrust { fork, trusted_head }) => {
                if (self.project && !fork) || trusted_head.as_ref() == Some(commit) {
                    AnalysisPolicy::EXECUTING
                } else {
                    AnalysisPolicy::LIMITED
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str) -> CommitId {
        CommitId(id.into())
    }

    fn pull_request(project: bool, fork: bool, trusted: Option<&str>) -> Trust {
        Trust {
            project,
            origin: Origin::PullRequest(PullRequestTrust {
                fork,
                trusted_head: trusted.map(commit),
            }),
        }
    }

    #[test]
    fn the_working_tree_and_branches_execute_servers_and_restores_only_in_a_trusted_project() {
        let local = |project| Trust {
            project,
            origin: Origin::Local,
        };
        assert_eq!(local(true).policy(&commit("a")), AnalysisPolicy::EXECUTING);
        assert_eq!(local(false).policy(&commit("a")), AnalysisPolicy::SERVERS);
    }

    #[test]
    fn a_pull_request_of_a_trusted_project_executes_unless_it_comes_from_a_fork() {
        let head = commit("a");
        assert_eq!(pull_request(true, false, None).policy(&head), AnalysisPolicy::EXECUTING);
        assert_eq!(pull_request(true, true, None).policy(&head), AnalysisPolicy::LIMITED);
        assert_eq!(pull_request(false, false, None).policy(&head), AnalysisPolicy::LIMITED);
        assert_eq!(pull_request(false, true, None).policy(&head), AnalysisPolicy::LIMITED);
    }

    #[test]
    fn trusting_a_pull_request_lifts_the_limit_for_that_head_only() {
        for (project, fork) in [(false, false), (false, true), (true, true)] {
            let trust = pull_request(project, fork, Some("a"));
            assert_eq!(
                trust.policy(&commit("a")),
                AnalysisPolicy::EXECUTING,
                "{project} {fork}"
            );
            assert_eq!(
                trust.policy(&commit("b")),
                AnalysisPolicy::LIMITED,
                "a new push voids the trust ({project} {fork})"
            );
        }
        // A trusted project's own pull requests do not lose their trust with a push.
        assert_eq!(
            pull_request(true, false, Some("a")).policy(&commit("b")),
            AnalysisPolicy::EXECUTING
        );
    }
}
