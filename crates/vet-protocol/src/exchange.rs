//! The request/response pairing of the protocol, by their tags.
//!
//! `ts-rs` exports the two enums; nothing in a type says which `Response` answers which
//! `Request`, so the shell paired them by hand. This is that pairing as data: the shell's
//! typed client and its runtime check are generated from it (`cargo test -p vet-protocol`
//! writes `ui/src/generated/ResponseOf.ts`), and the test below keeps it exhaustive over
//! both enums.

/// The `Response` tag each `Request` tag is answered with, in the order of `Request`.
pub const EXCHANGES: &[(&str, &str)] = &[
    ("ping", "pong"),
    ("subscribe", "done"),
    ("recentProjects", "recentProjects"),
    ("recordRecentProject", "recentProjects"),
    ("forgetRecentProject", "recentProjects"),
    ("accounts", "accounts"),
    ("setCredentials", "account"),
    ("removeCredentials", "accounts"),
    ("clearSemanticCaches", "semanticCacheUsage"),
    ("semanticCacheUsage", "semanticCacheUsage"),
    ("settings", "settings"),
    ("setSettings", "settings"),
    ("preferences", "preferences"),
    ("setPreference", "done"),
    ("logDirectory", "logDirectory"),
    ("openProject", "project"),
    ("closeProject", "done"),
    ("listChangesets", "changesets"),
    ("listPullRequests", "pullRequests"),
    ("changesetLines", "changesetLines"),
    ("checks", "checks"),
    ("checkLog", "checkLog"),
    ("pipelineSteps", "pipelineSteps"),
    ("changePipeline", "pipelineChanged"),
    ("openChangeset", "done"),
    ("finishReview", "reviewFinished"),
    ("tasks", "tasks"),
    ("versions", "versions"),
    ("changedSince", "changedFiles"),
    ("commits", "commits"),
    ("setLens", "done"),
    ("manifest", "manifest"),
    ("fileContents", "fileContents"),
    ("toggleHunk", "review"),
    ("toggleFlag", "review"),
    ("toggleItem", "review"),
    ("toggleFileSeen", "review"),
    ("setFilesSeen", "reviews"),
    ("reconfirmFile", "review"),
    ("baseDiff", "baseDiff"),
    ("itemBlob", "itemBlob"),
    ("setProjectFilter", "done"),
    ("narrowReview", "done"),
    ("setProjectTrusted", "done"),
    ("trustPullRequest", "done"),
    ("excludePath", "done"),
    ("fetch", "done"),
    ("listDirectory", "directory"),
    ("listFiles", "files"),
    ("patternMatches", "patternMatches"),
    ("fileText", "fileText"),
    ("search", "searchResults"),
    ("targetLine", "targetLine"),
    ("mapLines", "mappedLines"),
    ("threads", "threads"),
    ("commentTarget", "commentTarget"),
    ("createThread", "thread"),
    ("reply", "thread"),
    ("setThreadStatus", "thread"),
    ("deleteThread", "done"),
    ("editMessage", "thread"),
    ("removeMessage", "done"),
    ("keepEdit", "thread"),
    ("editConversationComment", "done"),
    ("deleteConversationComment", "done"),
    ("syncThreads", "done"),
    ("submitReview", "reviewSubmitted"),
    ("postDrafts", "reviewSubmitted"),
    ("stageVote", "done"),
    ("discardVote", "done"),
    ("pullRequestInfo", "pullRequestInfo"),
    ("markCommentsSeen", "done"),
    ("confirmAnchor", "thread"),
    ("attachAnchor", "thread"),
    ("hover", "hover"),
    ("definition", "definition"),
    ("references", "references"),
    ("implementations", "implementations"),
    ("diagnostics", "diagnostics"),
    ("fileSymbols", "fileSymbols"),
    ("fileOutline", "fileOutline"),
    ("workspaceSymbols", "workspaceSymbols"),
    ("symbolAt", "symbolAt"),
    ("signature", "signature"),
    ("calls", "calls"),
    ("usages", "usages"),
    ("removalImpact", "removalImpact"),
    ("reachability", "reachability"),
    ("listServers", "servers"),
    ("servedBy", "servedBy"),
    ("installServer", "servers"),
    ("setServerPath", "servers"),
    ("restartServer", "servers"),
    ("setReviewShown", "done"),
    ("setOpenFiles", "done"),
    ("refreshProblems", "done"),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// The tag of every variant of an enum, as serde's deserializer lists them.
    fn serde_tags<T: serde::de::DeserializeOwned + std::fmt::Debug>() -> BTreeSet<String> {
        let error = serde_json::from_value::<T>(serde_json::json!({ "type": "-" }))
            .unwrap_err()
            .to_string();
        error.split('`').skip(3).step_by(2).map(str::to_owned).collect()
    }

    /// `true` for every request tag the pairing names.
    fn request_tags() -> BTreeSet<String> {
        let tags = serde_tags::<crate::Request>();
        assert!(tags.contains("ping") && tags.contains("setServerPath"), "{tags:?}");
        tags
    }

    /// Likewise for the responses.
    fn response_tags() -> BTreeSet<String> {
        let tags = serde_tags::<crate::Response>();
        assert!(tags.contains("pong") && tags.contains("reviewSubmitted"), "{tags:?}");
        tags
    }

    fn ts_module() -> String {
        let mut out = String::from(
            "// This file was generated from `EXCHANGES` in crates/vet-protocol. Do not edit it manually.\n\
             \n\
             import type { Request } from \"./Request\";\n\
             import type { Response } from \"./Response\";\n\
             \n\
             /** The response tag each request tag is answered with, as the protocol pairs them. */\n\
             export const RESPONSE_TAG = {\n",
        );
        for (request, response) in EXCHANGES {
            out.push_str(&format!("  {request}: \"{response}\",\n"));
        }
        out.push_str(
            "} as const satisfies Record<Request[\"type\"], Response[\"type\"]>;\n\
             \n\
             /** The response the request of tag `T` is answered with. */\n\
             export type ResponseOf<T extends Request[\"type\"]> = Extract<Response, { type: (typeof RESPONSE_TAG)[T] }>;\n",
        );
        out
    }

    #[test]
    fn export_bindings_response_of() {
        crate::bindings::write("ResponseOf.ts", &ts_module());
    }

    #[test]
    fn the_pairing_covers_every_request_and_response() {
        let requests = request_tags();
        let responses = response_tags();
        let mut paired = BTreeSet::new();
        let mut answered = BTreeSet::new();
        for (request, response) in EXCHANGES {
            assert!(requests.contains(*request), "not a request tag: {request}");
            assert!(responses.contains(*response), "not a response tag: {response}");
            assert!(
                paired.insert((*request).to_owned()),
                "a request is paired twice: {request}"
            );
            answered.insert((*response).to_owned());
        }
        let missing: Vec<_> = requests.difference(&paired).collect();
        assert!(missing.is_empty(), "requests with no pairing: {missing:?}");
        let unused: Vec<_> = responses.difference(&answered).collect();
        assert!(unused.is_empty(), "responses no request is paired with: {unused:?}");
    }
}
