//! What analysis a caller permits. Computed by `vet-app` (Trust) from
//! PR trust and the head commit, so the pool never looks at provenance (I1).

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AnalysisPolicy {
    /// May language servers execute project code (build scripts, proc macros,
    /// MSBuild design-time builds)? `false` for untrusted contributor PRs (§7.5).
    pub allow_executing: bool,
}
