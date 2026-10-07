//! Shell <-> backend protocol (SPEC §4.1): one protocol over one loopback
//! WebSocket in every topology, JSON text frames.
//!
//! Ids and counts are `u32` so they map to plain TS `number`s.

mod envelope;
mod event;
mod file;
mod recent;
mod request;
mod response;
mod review;
mod semantic;
mod server;
mod text;
mod thread;

pub use envelope::*;
pub use event::*;
pub use file::*;
pub use recent::*;
pub use request::*;
pub use response::*;
pub use review::*;
pub use semantic::*;
pub use server::*;
pub use text::*;
pub use thread::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(response: &Response) -> serde_json::Value {
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(&serde_json::from_value::<Response>(json.clone()).unwrap(), response);
        json
    }

    #[test]
    fn file_contents_carry_highlights_of_both_sides() {
        let json = round_trip(&Response::FileContents {
            revision: 1,
            base: Some("let a = 1;".into()),
            target: None,
            changes: Vec::new(),
            base_highlights: vec![0, 3, 0, 5, 1, 2],
            target_highlights: Vec::new(),
        });
        assert_eq!(json["baseHighlights"], serde_json::json!([0, 3, 0, 5, 1, 2]));
        assert_eq!(json["targetHighlights"], serde_json::json!([]));
    }

    #[test]
    fn file_text_carries_highlights() {
        let json = round_trip(&Response::FileText {
            revision: 1,
            content: TextContent::Text {
                text: "fn f() {}".into(),
                highlights: vec![0, 2, 0],
            },
        });
        assert_eq!(json["content"]["highlights"], serde_json::json!([0, 2, 0]));
    }

    #[test]
    fn hover_carries_highlights_per_code_block() {
        let json = round_trip(&Response::Hover {
            revision: 1,
            side: Side::Target,
            text: Some("```rust\nfn f() {}\n```".into()),
            code_highlights: vec![vec![0, 2, 0], Vec::new()],
            unavailable: None,
        });
        assert_eq!(json["codeHighlights"], serde_json::json!([[0, 2, 0], []]));
    }
}
