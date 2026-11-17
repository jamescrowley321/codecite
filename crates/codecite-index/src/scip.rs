//! Reading SCIP indexes: the `.scip` files precise indexers (rust-analyzer,
//! scip-python, scip-typescript, scip-go, scip-dotnet) write. codecite
//! reads the file and never links an indexer's code.
//!
//! The messages below declare only the fields codecite reads, with the
//! field numbers of `scip.proto` (<https://github.com/sourcegraph/scip>);
//! protobuf skips every other field, so no generated code or `protoc` is
//! needed.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::Path;

use codecite_core::{ROLE_DEFINITION, ScipDefinition, ScipDocument, ScipOccurrence, symbol_name};
use prost::Message as _;

/// The largest SCIP index codecite reads: 256 MiB.
pub const MAX_SCIP_SIZE: u64 = 256 * 1024 * 1024;

/// The longest display name kept; a longer one gives way to the name the
/// symbol's descriptors give, so an index can't hand agents a long text as a
/// name.
const MAX_DISPLAY_NAME_BYTES: usize = 256;

/// The longest symbol string kept. Real symbols are far shorter; a longer
/// one is skipped rather than stored.
const MAX_SYMBOL_BYTES: usize = 1024;

/// Why a SCIP index couldn't be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ScipError {
    /// The file couldn't be opened or read.
    #[error("reading it failed")]
    Io(#[from] std::io::Error),
    /// It is a directory, FIFO or device rather than a file.
    #[error("it is not a regular file")]
    NotAFile,
    /// It is larger than [`MAX_SCIP_SIZE`].
    #[error("it is larger than the 256 MiB a SCIP index may be")]
    TooLarge,
    /// Its bytes are not a SCIP index.
    #[error("it is not a SCIP index")]
    Decode(#[from] prost::DecodeError),
}

/// `scip.Index`.
#[derive(Clone, PartialEq, prost::Message)]
struct Index {
    #[prost(message, repeated, tag = "2")]
    documents: Vec<Document>,
    #[prost(message, repeated, tag = "3")]
    external_symbols: Vec<SymbolInformation>,
}

/// `scip.Document`.
#[derive(Clone, PartialEq, prost::Message)]
struct Document {
    #[prost(string, tag = "1")]
    relative_path: String,
    #[prost(message, repeated, tag = "2")]
    occurrences: Vec<Occurrence>,
    #[prost(message, repeated, tag = "3")]
    symbols: Vec<SymbolInformation>,
}

/// `scip.Occurrence`. The typed ranges are a `oneof` in `scip.proto`; on
/// the wire a `oneof` is plain optional fields, so they are declared that
/// way.
#[derive(Clone, PartialEq, prost::Message)]
struct Occurrence {
    #[prost(int32, repeated, tag = "1")]
    range: Vec<i32>,
    #[prost(string, tag = "2")]
    symbol: String,
    #[prost(int32, tag = "3")]
    symbol_roles: i32,
    #[prost(int32, repeated, tag = "7")]
    enclosing_range: Vec<i32>,
    #[prost(message, optional, tag = "8")]
    single_line_range: Option<SingleLineRange>,
    #[prost(message, optional, tag = "9")]
    multi_line_range: Option<MultiLineRange>,
    #[prost(message, optional, tag = "10")]
    single_line_enclosing_range: Option<SingleLineRange>,
    #[prost(message, optional, tag = "11")]
    multi_line_enclosing_range: Option<MultiLineRange>,
}

/// `scip.SingleLineRange`: a half-open range on one line, from 0.
#[derive(Clone, PartialEq, prost::Message)]
struct SingleLineRange {
    #[prost(int32, tag = "1")]
    line: i32,
}

/// `scip.MultiLineRange`: a half-open range over lines, from 0.
#[derive(Clone, PartialEq, prost::Message)]
struct MultiLineRange {
    #[prost(int32, tag = "1")]
    start_line: i32,
    #[prost(int32, tag = "3")]
    end_line: i32,
    #[prost(int32, tag = "4")]
    end_character: i32,
}

/// `scip.SymbolInformation`. `kind` is the `SymbolInformation.Kind` enum,
/// read as its number.
#[derive(Clone, PartialEq, prost::Message)]
struct SymbolInformation {
    #[prost(string, tag = "1")]
    symbol: String,
    #[prost(int32, tag = "5")]
    kind: i32,
    #[prost(string, tag = "6")]
    display_name: String,
}

/// Reads the SCIP index at `path`: a regular file of at most
/// [`MAX_SCIP_SIZE`], with the read itself stopped past it.
pub fn read_scip(path: &Path) -> Result<Vec<ScipDocument>, ScipError> {
    decode_scip(&read_scip_bytes(path)?)
}

/// The bytes of the SCIP index at `path`: a regular file of at most
/// [`MAX_SCIP_SIZE`], with the read itself stopped past it.
pub fn read_scip_bytes(path: &Path) -> Result<Vec<u8>, ScipError> {
    // Checked before opening, since opening a FIFO waits for a writer, and
    // again after, in case the path changed in between.
    if !std::fs::metadata(path)?.is_file() {
        return Err(ScipError::NotAFile);
    }
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(ScipError::NotAFile);
    }
    let mut bytes = Vec::new();
    file.take(MAX_SCIP_SIZE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SCIP_SIZE {
        return Err(ScipError::TooLarge);
    }
    Ok(bytes)
}

/// Decodes a SCIP index into the documents codecite stores: each file's
/// global definitions and occurrences, with lines counted from 1. Local
/// symbols, which are unique only within a file, and occurrences with a
/// malformed range are left out.
pub fn decode_scip(bytes: &[u8]) -> Result<Vec<ScipDocument>, ScipError> {
    let index = Index::decode(bytes)?;
    let external: HashMap<&str, &SymbolInformation> = index
        .external_symbols
        .iter()
        .map(|i| (i.symbol.as_str(), i))
        .collect();
    let mut documents = Vec::with_capacity(index.documents.len());
    for doc in &index.documents {
        // The document's own information wins over the index's external
        // symbols, as `find` over the two in that order would.
        let mut own: HashMap<&str, &SymbolInformation> = HashMap::new();
        for i in &doc.symbols {
            own.entry(i.symbol.as_str()).or_insert(i);
        }
        let info = |symbol: &str| own.get(symbol).or_else(|| external.get(symbol)).copied();
        let path = doc
            .relative_path
            .strip_prefix("./")
            .unwrap_or(&doc.relative_path);
        let mut out = ScipDocument {
            path: path.to_owned(),
            ..ScipDocument::default()
        };
        for occ in &doc.occurrences {
            let symbol = occ.symbol.as_str();
            if symbol.is_empty() || symbol.starts_with("local ") || symbol.len() > MAX_SYMBOL_BYTES
            {
                continue;
            }
            let Some((start_line, end_line)) = occurrence_lines(occ) else {
                continue;
            };
            let roles = u32::try_from(occ.symbol_roles).unwrap_or_default();
            if roles & ROLE_DEFINITION != 0 {
                let found = info(symbol);
                let short = |n: &String| !n.is_empty() && n.len() <= MAX_DISPLAY_NAME_BYTES;
                let display_name = found
                    .map(|i| i.display_name.clone())
                    .filter(short)
                    .or_else(|| symbol_name(symbol).filter(short))
                    .unwrap_or_else(|| {
                        let name = symbol_name(symbol).unwrap_or_else(|| symbol.to_owned());
                        name[..name.floor_char_boundary(MAX_DISPLAY_NAME_BYTES)].to_owned()
                    });
                let (first, last) = enclosing_lines(occ).unwrap_or((start_line, end_line));
                out.definitions.push(ScipDefinition {
                    symbol: symbol.to_owned(),
                    display_name,
                    kind: kind_name(found.map_or(0, |i| i.kind), symbol).to_owned(),
                    line: start_line,
                    start_line: first.min(start_line),
                    end_line: last.max(start_line),
                });
            }
            out.occurrences.push(ScipOccurrence {
                symbol: symbol.to_owned(),
                start_line,
                end_line,
                roles,
            });
        }
        documents.push(out);
    }
    Ok(documents)
}

/// An occurrence's lines, from 1: its typed range, else the deprecated
/// `repeated int32` one.
fn occurrence_lines(occ: &Occurrence) -> Option<(u32, u32)> {
    if let Some(r) = &occ.single_line_range {
        return lines(r.line, r.line, 1);
    }
    if let Some(r) = &occ.multi_line_range {
        return lines(r.start_line, r.end_line, r.end_character);
    }
    packed_lines(&occ.range)
}

/// A definition's enclosing range as lines from 1, when the indexer gives
/// one.
fn enclosing_lines(occ: &Occurrence) -> Option<(u32, u32)> {
    if let Some(r) = &occ.single_line_enclosing_range {
        return lines(r.line, r.line, 1);
    }
    if let Some(r) = &occ.multi_line_enclosing_range {
        return lines(r.start_line, r.end_line, r.end_character);
    }
    packed_lines(&occ.enclosing_range)
}

/// The lines of a deprecated range: `[line, start, end]` on one line, or
/// `[start line, start, end line, end]`.
fn packed_lines(range: &[i32]) -> Option<(u32, u32)> {
    match *range {
        [line, _, _] => lines(line, line, 1),
        [start, _, end, end_character] => lines(start, end, end_character),
        _ => None,
    }
}

/// A half-open range from 0 as inclusive lines from 1. A range that ends at
/// the start of a later line ends on the line before it.
fn lines(start: i32, end: i32, end_character: i32) -> Option<(u32, u32)> {
    let start = u32::try_from(start).ok()?;
    let mut end = u32::try_from(end).ok()?;
    if end < start {
        return None;
    }
    if end_character == 0 && end > start {
        end -= 1;
    }
    Some((start.checked_add(1)?, end.checked_add(1)?))
}

/// The kind codecite stores for a SCIP definition: from its
/// `SymbolInformation.Kind` when it has a common one, else from its last
/// descriptor's suffix.
fn kind_name(kind: i32, symbol: &str) -> &'static str {
    match kind {
        7 => "class",
        8 => "constant",
        9 => "constructor",
        11 => "enum",
        12 => "enum_member",
        15 | 79 => "field",
        17 => "function",
        21 => "interface",
        25 => "macro",
        26 | 66 | 70 | 80 => "method",
        29 | 30 | 35 => "module",
        41 | 81 => "property",
        49 => "struct",
        53 => "trait",
        54 => "type",
        55 => "type_alias",
        61 | 82 => "variable",
        _ => match symbol.chars().last() {
            Some('/') => "module",
            Some('#') => "type",
            Some('!') => "macro",
            Some('.') if symbol.ends_with(").") => "method",
            Some('.') => "term",
            Some(')') => "parameter",
            Some(']') => "type_parameter",
            _ => "symbol",
        },
    }
}

/// `bytes`, a SCIP index, without its document at `path`, for tests that
/// change an index.
#[cfg(test)]
pub(crate) fn without_document(bytes: &[u8], path: &str) -> Vec<u8> {
    let mut index = Index::decode(bytes).unwrap();
    index.documents.retain(|d| d.relative_path != path);
    index.encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/scip/app/index.scip"
    );
    const PKG: &str = "rust-analyzer cargo app 0.1.0 ";

    fn doc<'a>(docs: &'a [ScipDocument], path: &str) -> &'a ScipDocument {
        docs.iter().find(|d| d.path == path).unwrap()
    }

    #[test]
    fn reads_rust_analyzers_index_of_the_fixture() {
        let docs = read_scip(Path::new(FIXTURE)).unwrap();
        let mut paths: Vec<&str> = docs.iter().map(|d| d.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(paths, ["src/client.rs", "src/lib.rs"]);

        let client = doc(&docs, "src/client.rs");
        let defs: Vec<(&str, &str, &str, u32, u32, u32)> = client
            .definitions
            .iter()
            .map(|d| {
                (
                    d.symbol.strip_prefix(PKG).unwrap(),
                    d.display_name.as_str(),
                    d.kind.as_str(),
                    d.line,
                    d.start_line,
                    d.end_line,
                )
            })
            .collect();
        assert_eq!(
            defs,
            [
                ("client/", "client", "module", 1, 1, 22),
                ("client/Client#", "Client", "struct", 2, 1, 4),
                ("client/Client#issuer.", "issuer", "field", 3, 3, 3),
                ("client/impl#[Client]new().", "new", "method", 8, 7, 12),
                (
                    "client/impl#[Client]validate().",
                    "validate",
                    "method",
                    15,
                    14,
                    17
                ),
                ("client/check().", "check", "function", 20, 20, 22),
            ]
        );
        // No local symbol (parameters, `self`) is kept.
        assert!(
            client
                .occurrences
                .iter()
                .all(|o| !o.symbol.starts_with("local"))
        );
        let calls: Vec<u32> = client
            .occurrences
            .iter()
            .filter(|o| o.symbol == format!("{PKG}client/check().") && o.roles == 0)
            .map(|o| o.start_line)
            .collect();
        assert_eq!(calls, [16]);

        let lib = doc(&docs, "src/lib.rs");
        let logins: Vec<(u32, u32)> = lib
            .occurrences
            .iter()
            .filter(|o| o.symbol == format!("{PKG}login()."))
            .map(|o| (o.start_line, o.roles))
            .collect();
        assert_eq!(logins, [(8, 1), (15, 0), (15, 0)]);
    }

    #[test]
    fn typed_ranges_win_over_the_deprecated_ones() {
        let occurrence = Occurrence {
            range: vec![0, 0, 3],
            symbol: format!("{PKG}f()."),
            symbol_roles: 1,
            enclosing_range: vec![0, 0, 9, 1],
            single_line_range: Some(SingleLineRange { line: 4 }),
            multi_line_range: None,
            single_line_enclosing_range: None,
            multi_line_enclosing_range: Some(MultiLineRange {
                start_line: 3,
                end_line: 7,
                end_character: 0,
            }),
        };
        let index = Index {
            documents: vec![Document {
                relative_path: "./src/f.rs".into(),
                occurrences: vec![occurrence],
                symbols: vec![],
            }],
            external_symbols: vec![],
        };
        let docs = decode_scip(&index.encode_to_vec()).unwrap();
        assert_eq!(docs[0].path, "src/f.rs");
        let def = &docs[0].definitions[0];
        // Line 4 from 0 is line 5; the enclosing range ends at the start of
        // line 7, so on line 7 counted from 1.
        assert_eq!((def.line, def.start_line, def.end_line), (5, 4, 7));
        assert_eq!(def.display_name, "f");
        assert_eq!(def.kind, "method");
    }

    #[test]
    fn a_long_display_name_gives_way_to_the_descriptors_name() {
        let def = |symbol: &str| Occurrence {
            range: vec![0, 0, 3],
            symbol: symbol.into(),
            symbol_roles: 1,
            enclosing_range: vec![],
            single_line_range: None,
            multi_line_range: None,
            single_line_enclosing_range: None,
            multi_line_enclosing_range: None,
        };
        let info = |symbol: &str, name: String| SymbolInformation {
            symbol: symbol.into(),
            kind: 17,
            display_name: name,
        };
        let (long, short) = (format!("{PKG}f()."), format!("{PKG}g()."));
        let backticked = format!("{PKG}`{}`().", "é ".repeat(300));
        let index = Index {
            documents: vec![Document {
                relative_path: "a.rs".into(),
                occurrences: vec![def(&long), def(&short), def(&backticked)],
                symbols: vec![info(&long, "x".repeat(300)), info(&short, "gee".into())],
            }],
            external_symbols: vec![info(&short, "external".into())],
        };
        let docs = decode_scip(&index.encode_to_vec()).unwrap();
        let names: Vec<&str> = docs[0]
            .definitions
            .iter()
            .map(|d| d.display_name.as_str())
            .collect();
        assert_eq!(names[..2], ["f", "gee"]);
        // A descriptor's name is held to the same cap.
        assert!(
            names[2].len() <= MAX_DISPLAY_NAME_BYTES && names[2].starts_with("é "),
            "{}",
            names[2]
        );
    }

    #[test]
    fn malformed_ranges_and_long_symbols_are_skipped() {
        let occ = |range: Vec<i32>, symbol: String| Occurrence {
            range,
            symbol,
            symbol_roles: 0,
            enclosing_range: vec![],
            single_line_range: None,
            multi_line_range: None,
            single_line_enclosing_range: None,
            multi_line_enclosing_range: None,
        };
        let index = Index {
            documents: vec![Document {
                relative_path: "a.rs".into(),
                occurrences: vec![
                    occ(vec![1, 2], format!("{PKG}a().")),
                    occ(vec![-1, 0, 3], format!("{PKG}a().")),
                    occ(vec![5, 0, 2, 3], format!("{PKG}a().")),
                    occ(vec![1, 0, 3], format!("{PKG}{}.", "x".repeat(2000))),
                    occ(vec![1, 0, 3], String::new()),
                    occ(vec![2, 0, 3], format!("{PKG}a().")),
                ],
                symbols: vec![],
            }],
            external_symbols: vec![],
        };
        let docs = decode_scip(&index.encode_to_vec()).unwrap();
        let kept: Vec<u32> = docs[0].occurrences.iter().map(|o| o.start_line).collect();
        assert_eq!(kept, [3]);
    }

    #[test]
    fn bad_files_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_scip(dir.path()).unwrap_err();
        assert!(matches!(err, ScipError::NotAFile), "{err:?}");
        // A FIFO is refused without waiting for a writer.
        #[cfg(unix)]
        {
            let fifo = dir.path().join("fifo.scip");
            let made = std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap();
            assert!(made.success());
            let err = read_scip(&fifo).unwrap_err();
            assert!(matches!(err, ScipError::NotAFile), "{err:?}");
        }
        let truncated = dir.path().join("cut.scip");
        let bytes = std::fs::read(FIXTURE).unwrap();
        std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
        let err = read_scip(&truncated).unwrap_err();
        assert!(matches!(err, ScipError::Decode(_)), "{err:?}");
        let err = read_scip(&dir.path().join("missing.scip")).unwrap_err();
        assert!(matches!(err, ScipError::Io(_)), "{err:?}");
    }

    #[test]
    fn kinds_fall_back_to_the_descriptor_suffix() {
        assert_eq!(kind_name(17, "a b c d f()."), "function");
        assert_eq!(kind_name(0, "a b c d m/"), "module");
        assert_eq!(kind_name(0, "a b c d T#"), "type");
        assert_eq!(kind_name(0, "a b c d T#x."), "term");
        assert_eq!(kind_name(0, "a b c d T#f()."), "method");
        assert_eq!(kind_name(0, "a b c d m!"), "macro");
    }
}
