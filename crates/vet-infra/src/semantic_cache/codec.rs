//! The stored form of an answer and the address of the row it is kept under.
//!
//! An answer is stored as the semantic layer has it, before any Vet logic is applied
//! to it (`symbols::flatten`, the preference for locations inside the root), so a
//! change to that logic never needs the cache flushed. A value that does not decode
//! to exactly the kind its key names is corrupt.
//!
//! Each kind has its own format version, which is part of the address: a change to
//! how one kind is stored makes its old rows unreachable, and the least recently used
//! eviction removes them. The other kinds keep theirs.

use vet_core::content::{Hash, RepoPath};
use vet_semantic::ports::{At, Diagnostic, Location, Pos, ServerIdentity, Severity, Span, Sym, SymbolInfo};
use vet_semantic::reuse::{Key, ProjectQuery, Restore, Value};

use crate::restore::record::Digest;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    Symbols,
    Locations,
    Hover,
    Diagnostics,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Symbols => "symbols",
            Kind::Locations => "locations",
            Kind::Hover => "hover",
            Kind::Diagnostics => "diagnostics",
        }
    }

    /// Bumped when the stored form of this kind changes.
    fn format(self) -> u32 {
        match self {
            Kind::Symbols => 2,
            Kind::Locations => 1,
            Kind::Hover => 1,
            Kind::Diagnostics => 1,
        }
    }

    pub(super) fn id(self) -> i64 {
        match self {
            Kind::Symbols => 1,
            Kind::Locations => 2,
            Kind::Hover => 3,
            Kind::Diagnostics => 4,
        }
    }
}

pub(super) type Address = [u8; 32];

/// The kind of answer a key asks for; none for a question that is never kept.
fn kind_of(key: &Key) -> Option<Kind> {
    let Key::Project { query, .. } = key else {
        return Some(Kind::Symbols);
    };
    match query {
        ProjectQuery::DocumentSymbols(_) | ProjectQuery::Incoming(_) | ProjectQuery::Outgoing(_) => Some(Kind::Symbols),
        ProjectQuery::Definition(_) | ProjectQuery::References(_) | ProjectQuery::Implementations(_) => {
            Some(Kind::Locations)
        }
        ProjectQuery::Hover(_) => Some(Kind::Hover),
        ProjectQuery::Diagnostics(_) => Some(Kind::Diagnostics),
        ProjectQuery::Workspace(_) => None,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn identity(digest: &mut Digest, server: &ServerIdentity) {
    digest.field(&server.backend.0).field(&server.fingerprint.to_hex());
}

fn at(digest: &mut Digest, at: &At) {
    digest
        .field(&hex(&at.file.0))
        .field(&at.pos.line.to_string())
        .field(&at.pos.col.to_string());
}

/// Where the answer under `key` is kept, and what it is; none if it is not kept at all.
/// Two keys have one address only if they are equal. The label of the server is not
/// part of it: the fingerprint says everything the label shows.
pub(super) fn address(key: &Key) -> Option<(Address, Kind)> {
    if !key.persists() {
        return None;
    }
    let kind = kind_of(key)?;
    let mut digest = Digest::default();
    digest
        .field("semantic answer")
        .field(kind.name())
        .field(&kind.format().to_string());
    match key {
        Key::Text {
            server,
            extension,
            blob,
        } => {
            digest.field("text");
            identity(&mut digest, server);
            digest.field(&hex(extension)).field(&blob.0.to_hex());
        }
        Key::Project { server, basis, query } => {
            digest.field("project");
            identity(&mut digest, server);
            digest
                .field(&basis.snapshot.0.to_hex())
                .field(if basis.policy.allow_executing {
                    "executing"
                } else {
                    "inert"
                });
            let Restore::Recorded(restore) = basis.restore else {
                return None;
            };
            digest.field(&restore.to_hex());
            match query {
                ProjectQuery::DocumentSymbols(file) => {
                    digest.field("document symbols").field(&hex(&file.0));
                }
                ProjectQuery::Definition(place) => {
                    digest.field("definition");
                    at(&mut digest, place);
                }
                ProjectQuery::References(place) => {
                    digest.field("references");
                    at(&mut digest, place);
                }
                ProjectQuery::Implementations(place) => {
                    digest.field("implementations");
                    at(&mut digest, place);
                }
                ProjectQuery::Hover(place) => {
                    digest.field("hover");
                    at(&mut digest, place);
                }
                ProjectQuery::Incoming(place) => {
                    digest.field("incoming");
                    at(&mut digest, place);
                }
                ProjectQuery::Outgoing(place) => {
                    digest.field("outgoing");
                    at(&mut digest, place);
                }
                ProjectQuery::Diagnostics(file) => {
                    digest.field("diagnostics").field(&hex(&file.0));
                }
                ProjectQuery::Workspace(_) => return None,
            }
        }
    }
    let Hash(address) = digest.finish();
    Some((address, kind))
}

fn kind_of_value(value: &Value) -> Kind {
    match value {
        Value::Symbols(_) => Kind::Symbols,
        Value::Locations(_) => Kind::Locations,
        Value::Hover(_) => Kind::Hover,
        Value::Diagnostics(_) => Kind::Diagnostics,
    }
}

struct Writer(Vec<u8>);

impl Writer {
    fn number(&mut self, n: u32) {
        self.0.extend(n.to_le_bytes());
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.number(bytes.len() as u32);
        self.0.extend(bytes);
    }

    fn text(&mut self, text: &str) {
        self.bytes(text.as_bytes());
    }

    fn maybe(&mut self, text: &Option<String>) {
        match text {
            Some(text) => {
                self.0.push(1);
                self.text(text);
            }
            None => self.0.push(0),
        }
    }

    fn span(&mut self, span: Span) {
        for n in [span.start.line, span.start.col, span.end.line, span.end.col] {
            self.number(n);
        }
    }

    fn symbol(&mut self, symbol: &SymbolInfo) {
        self.bytes(&symbol.sym.file.0);
        self.number(symbol.sym.name.len() as u32);
        symbol.sym.name.iter().for_each(|part| self.text(part));
        self.span(symbol.span);
        self.number(symbol.at.line);
        self.number(symbol.at.col);
        self.maybe(&symbol.detail);
        self.0.push(u8::from(symbol.container));
    }
}

pub(super) fn encode(value: &Value) -> (Kind, Vec<u8>) {
    let kind = kind_of_value(value);
    let mut out = Writer(Vec::new());
    match value {
        Value::Symbols(symbols) => {
            out.number(symbols.len() as u32);
            symbols.iter().for_each(|s| out.symbol(s));
        }
        Value::Locations(locations) => {
            out.number(locations.len() as u32);
            for location in locations {
                out.bytes(&location.file.0);
                out.span(location.span);
                out.0.push(u8::from(location.external));
            }
        }
        Value::Hover(text) => out.maybe(text),
        Value::Diagnostics(diagnostics) => {
            out.number(diagnostics.len() as u32);
            for d in diagnostics {
                out.span(d.span);
                out.0.push(match d.severity {
                    Severity::Error => 0,
                    Severity::Warning => 1,
                    Severity::Info => 2,
                    Severity::Hint => 3,
                });
                out.text(&d.message);
                out.maybe(&d.source);
                out.maybe(&d.code);
            }
        }
    }
    (kind, out.0)
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }

    fn number(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn bytes(&mut self) -> Option<Vec<u8>> {
        let len = self.number()? as usize;
        Some(self.take(len)?.to_vec())
    }

    fn text(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?).ok()
    }

    fn flag(&mut self) -> Option<bool> {
        match self.take(1)?[0] {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    fn maybe(&mut self) -> Option<Option<String>> {
        if self.flag()? {
            Some(Some(self.text()?))
        } else {
            Some(None)
        }
    }

    fn span(&mut self) -> Option<Span> {
        Some(Span {
            start: Pos {
                line: self.number()?,
                col: self.number()?,
            },
            end: Pos {
                line: self.number()?,
                col: self.number()?,
            },
        })
    }

    /// The items a leading count announces. A count that outruns the input fails at the first item missing.
    fn each<T>(&mut self, mut item: impl FnMut(&mut Self) -> Option<T>) -> Option<Vec<T>> {
        let count = self.number()?;
        let mut items = Vec::new();
        for _ in 0..count {
            items.push(item(self)?);
        }
        Some(items)
    }

    fn symbol(&mut self) -> Option<SymbolInfo> {
        Some(SymbolInfo {
            sym: Sym {
                file: RepoPath(self.bytes()?),
                name: self.each(Self::text)?,
            },
            span: self.span()?,
            at: Pos {
                line: self.number()?,
                col: self.number()?,
            },
            detail: self.maybe()?,
            container: self.flag()?,
        })
    }
}

/// The value of `kind` stored as `bytes`; none unless they are exactly one such value.
pub(super) fn decode(kind: Kind, bytes: &[u8]) -> Option<Value> {
    let mut input = Reader(bytes);
    let value = match kind {
        Kind::Symbols => Value::Symbols(input.each(Reader::symbol)?),
        Kind::Locations => Value::Locations(input.each(|r| {
            Some(Location {
                file: RepoPath(r.bytes()?),
                span: r.span()?,
                external: r.flag()?,
            })
        })?),
        Kind::Hover => Value::Hover(input.maybe()?),
        Kind::Diagnostics => Value::Diagnostics(input.each(|r| {
            Some(Diagnostic {
                span: r.span()?,
                severity: match r.take(1)?[0] {
                    0 => Severity::Error,
                    1 => Severity::Warning,
                    2 => Severity::Info,
                    3 => Severity::Hint,
                    _ => return None,
                },
                message: r.text()?,
                source: r.maybe()?,
                code: r.maybe()?,
            })
        })?),
    };
    input.0.is_empty().then_some(value)
}

#[cfg(test)]
mod tests;
