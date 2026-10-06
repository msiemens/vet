//! Execution and trust (§7.5, §9). Computes the `AnalysisPolicy` handed to the
//! pool, so `vet-semantic` never looks at provenance. Executing restores and
//! launches go through the run check here, before the pool or a process is asked.

use vet_semantic::policy::AnalysisPolicy;

use crate::pin::CommitId;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Trust {
    /// A local dirty tree or a branch of your own.
    Local,
    /// A contributor PR: trusted for exactly one head commit, so any new push
    /// voids the trust.
    Contributor { trusted_head: Option<CommitId> },
}

pub fn analysis_policy(trust: &Trust, head: &CommitId) -> AnalysisPolicy {
    let allow_executing = match trust {
        Trust::Local => true,
        Trust::Contributor { trusted_head } => trusted_head.as_ref() == Some(head),
    };
    AnalysisPolicy { allow_executing }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str) -> CommitId {
        CommitId(id.into())
    }

    #[test]
    fn a_new_push_voids_trust() {
        let trust = Trust::Contributor {
            trusted_head: Some(commit("a")),
        };
        assert!(analysis_policy(&trust, &commit("a")).allow_executing);
        assert!(!analysis_policy(&trust, &commit("b")).allow_executing);
        assert!(!analysis_policy(&Trust::Contributor { trusted_head: None }, &commit("a")).allow_executing);
        assert!(analysis_policy(&Trust::Local, &commit("a")).allow_executing);
    }
}
