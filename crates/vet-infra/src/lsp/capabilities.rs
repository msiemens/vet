//! The required operations (§7.1), asserted on the server's `initialize`
//! result. A missing one means the language is unsupported (I6).

use lsp_types::{CallHierarchyServerCapability, HoverProviderCapability, OneOf, ServerCapabilities};
use vet_semantic::ports::LaunchError;

/// How diagnostics are obtained.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Diagnostics {
    /// `textDocument/diagnostic`.
    Pull,
    /// `textDocument/publishDiagnostics`: servers do not announce it, so it cannot be asserted.
    Push,
}

fn one_of<T>(cap: &Option<OneOf<bool, T>>) -> bool {
    !matches!(cap, None | Some(OneOf::Left(false)))
}

pub(super) fn assert_capabilities(caps: &ServerCapabilities) -> Result<Diagnostics, LaunchError> {
    let missing = |name| Err(LaunchError::MissingCapability(name));
    if !one_of(&caps.document_symbol_provider) {
        return missing("textDocument/documentSymbol");
    }
    if !one_of(&caps.definition_provider) {
        return missing("textDocument/definition");
    }
    if !one_of(&caps.references_provider) {
        return missing("textDocument/references");
    }
    if matches!(caps.hover_provider, None | Some(HoverProviderCapability::Simple(false))) {
        return missing("textDocument/hover");
    }
    if matches!(
        caps.call_hierarchy_provider,
        None | Some(CallHierarchyServerCapability::Simple(false))
    ) {
        return missing("callHierarchy (incoming and outgoing calls)");
    }
    if !one_of(&caps.workspace_symbol_provider) {
        return missing("workspace/symbol");
    }
    Ok(if caps.diagnostic_provider.is_some() {
        Diagnostics::Pull
    } else {
        Diagnostics::Push
    })
}

#[cfg(test)]
mod tests {
    use lsp_types::DiagnosticServerCapabilities;

    use super::*;

    fn full() -> ServerCapabilities {
        ServerCapabilities {
            document_symbol_provider: Some(OneOf::Left(true)),
            definition_provider: Some(OneOf::Left(true)),
            references_provider: Some(OneOf::Left(true)),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
            workspace_symbol_provider: Some(OneOf::Left(true)),
            ..Default::default()
        }
    }

    fn missing(caps: &ServerCapabilities) -> &'static str {
        match assert_capabilities(caps) {
            Err(LaunchError::MissingCapability(name)) => name,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn complete_server_is_accepted_with_push_diagnostics_by_default() {
        assert_eq!(assert_capabilities(&full()).unwrap(), Diagnostics::Push);
        let mut caps = full();
        caps.diagnostic_provider = Some(DiagnosticServerCapabilities::Options(Default::default()));
        assert_eq!(assert_capabilities(&caps).unwrap(), Diagnostics::Pull);
    }

    #[test]
    fn each_missing_operation_is_named() {
        let mut caps = full();
        caps.document_symbol_provider = None;
        assert_eq!(missing(&caps), "textDocument/documentSymbol");
        let mut caps = full();
        caps.definition_provider = Some(OneOf::Left(false));
        assert_eq!(missing(&caps), "textDocument/definition");
        let mut caps = full();
        caps.references_provider = None;
        assert_eq!(missing(&caps), "textDocument/references");
        let mut caps = full();
        caps.hover_provider = None;
        assert_eq!(missing(&caps), "textDocument/hover");
        let mut caps = full();
        caps.call_hierarchy_provider = None;
        assert!(missing(&caps).starts_with("callHierarchy"));
        let mut caps = full();
        caps.workspace_symbol_provider = None;
        assert_eq!(missing(&caps), "workspace/symbol");
    }
}
