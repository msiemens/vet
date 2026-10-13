//! What a server offers (§7.1), read from its `initialize` result. A server that
//! cannot be fed documents cannot work with Vet at all and is refused; every other
//! operation is optional, and a query for one the server lacks is answered with
//! "not offered" (I6), never with an empty answer.

use lsp_types::{
    CallHierarchyServerCapability, HoverProviderCapability, ImplementationProviderCapability, OneOf,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind,
};
use vet_semantic::ports::{Feature, Features, LaunchError};

/// How diagnostics are obtained.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Diagnostics {
    /// `textDocument/diagnostic`.
    Pull,
    /// `textDocument/publishDiagnostics`: servers do not announce it, so it cannot be asserted.
    Push,
}

/// What a server offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Offered {
    pub diagnostics: Diagnostics,
    pub features: Features,
}

fn one_of<T>(cap: &Option<OneOf<bool, T>>) -> bool {
    !matches!(cap, None | Some(OneOf::Left(false)))
}

/// Whether the server takes the full text of changed documents, which is how Vet keeps it in step
/// with the disk.
fn follows_documents(sync: &Option<TextDocumentSyncCapability>) -> bool {
    match sync {
        None => false,
        Some(TextDocumentSyncCapability::Kind(kind)) => *kind != TextDocumentSyncKind::NONE,
        Some(TextDocumentSyncCapability::Options(options)) => {
            options.open_close != Some(false) && !matches!(options.change, None | Some(TextDocumentSyncKind::NONE))
        }
    }
}

pub(super) fn read_capabilities(caps: &ServerCapabilities) -> Result<Offered, LaunchError> {
    if !follows_documents(&caps.text_document_sync) {
        return Err(LaunchError::MissingCapability("text document synchronization"));
    }
    let features = Features::NONE
        .with(Feature::DocumentSymbols, one_of(&caps.document_symbol_provider))
        .with(Feature::Definition, one_of(&caps.definition_provider))
        .with(Feature::References, one_of(&caps.references_provider))
        .with(
            Feature::Implementations,
            !matches!(
                caps.implementation_provider,
                None | Some(ImplementationProviderCapability::Simple(false))
            ),
        )
        .with(
            Feature::Hover,
            !matches!(caps.hover_provider, None | Some(HoverProviderCapability::Simple(false))),
        )
        .with(
            Feature::CallHierarchy,
            !matches!(
                caps.call_hierarchy_provider,
                None | Some(CallHierarchyServerCapability::Simple(false))
            ),
        )
        .with(Feature::WorkspaceSymbols, one_of(&caps.workspace_symbol_provider));
    Ok(Offered {
        diagnostics: if caps.diagnostic_provider.is_some() {
            Diagnostics::Pull
        } else {
            Diagnostics::Push
        },
        features,
    })
}

#[cfg(test)]
mod tests {
    use lsp_types::{DiagnosticServerCapabilities, StaticTextDocumentRegistrationOptions, TextDocumentSyncOptions};

    use super::*;

    fn synced() -> ServerCapabilities {
        ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
            ..Default::default()
        }
    }

    fn full() -> ServerCapabilities {
        ServerCapabilities {
            document_symbol_provider: Some(OneOf::Left(true)),
            definition_provider: Some(OneOf::Left(true)),
            references_provider: Some(OneOf::Left(true)),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
            workspace_symbol_provider: Some(OneOf::Left(true)),
            implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
            ..synced()
        }
    }

    fn features(caps: &ServerCapabilities) -> Features {
        read_capabilities(caps).unwrap().features
    }

    #[test]
    fn diagnostics_are_pushed_unless_the_server_announces_pull() {
        assert_eq!(read_capabilities(&full()).unwrap().diagnostics, Diagnostics::Push);
        let mut caps = full();
        caps.diagnostic_provider = Some(DiagnosticServerCapabilities::Options(Default::default()));
        assert_eq!(read_capabilities(&caps).unwrap().diagnostics, Diagnostics::Pull);
    }

    #[test]
    fn a_server_offering_nothing_but_document_sync_is_accepted_and_offers_no_feature() {
        assert_eq!(features(&synced()), Features::NONE);
        assert_eq!(features(&full()), Features::ALL);
    }

    #[test]
    fn each_feature_is_offered_only_when_the_server_advertises_it() {
        type Remove = fn(&mut ServerCapabilities);
        let lacking: [(Feature, Remove); 7] = [
            (Feature::DocumentSymbols, |c| c.document_symbol_provider = None),
            (Feature::Definition, |c| {
                c.definition_provider = Some(OneOf::Left(false))
            }),
            (Feature::References, |c| c.references_provider = None),
            (Feature::Hover, |c| {
                c.hover_provider = Some(HoverProviderCapability::Simple(false))
            }),
            (Feature::CallHierarchy, |c| c.call_hierarchy_provider = None),
            (Feature::WorkspaceSymbols, |c| c.workspace_symbol_provider = None),
            (Feature::Implementations, |c| c.implementation_provider = None),
        ];
        for (feature, remove) in lacking {
            let mut caps = full();
            remove(&mut caps);
            let offered = features(&caps);
            for other in Feature::ALL {
                assert_eq!(
                    offered.offers(other),
                    other != feature,
                    "{feature:?} removed, {other:?}"
                );
            }
        }
    }

    #[test]
    fn implementation_options_count_as_offered() {
        let mut caps = synced();
        caps.implementation_provider = Some(ImplementationProviderCapability::Options(
            StaticTextDocumentRegistrationOptions {
                document_selector: None,
                id: None,
            },
        ));
        assert!(features(&caps).offers(Feature::Implementations));
    }

    #[test]
    fn a_server_that_does_not_follow_documents_is_refused() {
        let refused = |caps: ServerCapabilities| match read_capabilities(&caps) {
            Err(LaunchError::MissingCapability(name)) => name,
            other => panic!("{other:?}"),
        };
        let mut caps = full();
        caps.text_document_sync = None;
        assert!(refused(caps).contains("synchronization"));
        let mut caps = full();
        caps.text_document_sync = Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::NONE));
        refused(caps);
        let mut caps = full();
        caps.text_document_sync = Some(TextDocumentSyncCapability::Options(TextDocumentSyncOptions {
            open_close: Some(true),
            change: None,
            ..Default::default()
        }));
        refused(caps);
        let mut caps = full();
        caps.text_document_sync = Some(TextDocumentSyncCapability::Options(TextDocumentSyncOptions {
            open_close: Some(true),
            change: Some(TextDocumentSyncKind::INCREMENTAL),
            ..Default::default()
        }));
        assert!(read_capabilities(&caps).is_ok());
    }
}
