//! What analysis a caller permits. Computed by `vet-app` (Trust) from
//! project and PR trust and the commit analysed, so the pool never looks at provenance.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AnalysisPolicy {
    /// May language servers execute project code (build scripts, proc macros,
    /// MSBuild design-time builds)? `false` for what is not trusted (§6.5).
    pub allow_executing: bool,
    /// May a restore execute project code (install scripts, build backends, MSBuild)?
    /// Never set without `allow_executing`.
    pub allow_executing_restores: bool,
}

impl AnalysisPolicy {
    /// Nothing executes: fetch-only restores, and only servers that do not execute project code.
    pub const LIMITED: Self = Self {
        allow_executing: false,
        allow_executing_restores: false,
    };

    /// Servers may execute project code; restores stay fetch-only.
    pub const SERVERS: Self = Self {
        allow_executing: true,
        allow_executing_restores: false,
    };

    /// Servers and restores may execute project code.
    pub const EXECUTING: Self = Self {
        allow_executing: true,
        allow_executing_restores: true,
    };

    /// May a language server load JSON schemas the repository names (`$schema`, a modeline) or
    /// the schema store from the network? A repository could otherwise make the machine send
    /// requests, so this is for what is trusted to execute restores; the rest checks against
    /// the schemas Vet bundles.
    pub fn allow_schema_downloads(self) -> bool {
        self.allow_executing_restores
    }

    /// Names the policy in the keys of what depends on all of it (cached answers): two
    /// policies share a name only if they permit the same.
    pub fn tag(self) -> &'static str {
        match (self.allow_executing, self.allow_executing_restores) {
            (false, false) => "noexec",
            (true, false) => "servers",
            (true, true) => "exec",
            (false, true) => "restores",
        }
    }

    /// What a restore under this policy may do, which is all that decides what a kept tree
    /// holds: policies that restore alike share a tree and its restore records.
    pub fn restore_kind(self) -> RestoreKind {
        if self.allow_executing_restores {
            RestoreKind::Exec
        } else {
            RestoreKind::Fetch
        }
    }
}

/// Whether a restore may execute project code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RestoreKind {
    /// Fetch-only: no install scripts or build steps.
    Fetch,
    /// Install scripts and build steps run.
    Exec,
}

impl RestoreKind {
    pub fn tag(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Exec => "exec",
        }
    }
}
