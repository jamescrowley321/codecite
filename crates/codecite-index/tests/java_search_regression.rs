//! A long Java class must remain discoverable alongside short constructors.
use codecite_core::{ChunkRecord, DocumentRecord, SearchQuery, SourceRecord, Store, SymbolRecord};
use codecite_index::{CHUNKER_VERSION, Chunker};

fn java_class_with_constructors() -> String {
    let mut source = String::from(
        "public class TokenEndpoint {\n\
         public TokenEndpoint() {}\n\
         public TokenEndpoint(String name) {}\n\
         public TokenEndpoint(int code) {}\n",
    );
    // Long bodies and many same-file matches reproduce the ranking shape of a
    // real endpoint without including another project's source in the repo.
    for i in 0..400 {
        source.push_str(&format!("private String field{i} = \"example\";\n"));
    }
    for i in 0..25 {
        source.push_str(&format!("public void handler{i}() {{}}\n"));
    }
    source.push_str("}\n");
    source
}

#[test]
fn exact_class_search_returns_declaration_among_short_same_file_matches() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
    let source_id = store
        .upsert_source(&SourceRecord {
            name: "demo",
            kind: "local",
            url: "/demo",
            revision: "0123456789abcdef0123456789abcdef01234567",
            license_spdx: Some("MIT"),
            license_class: "adapt",
            is_current: true,
            obsoleted_by: None,
            updated_by: None,
            role_globs: None,
        })
        .unwrap();
    let text = java_class_with_constructors();
    let path = "src/TokenEndpoint.java";
    let chunks = Chunker::new().unwrap().chunk(path, &text);
    let document = DocumentRecord {
        path,
        language: Some("java"),
        content_hash: b"test-source",
        size_bytes: text.len() as u64,
        dirty: false,
        text: Some(&text),
        chunks: chunks
            .iter()
            .map(|c| ChunkRecord {
                kind: c.kind.as_str(),
                start_line: c.start_line,
                end_line: c.end_line,
                text: c.text,
                number: None,
                anchor: c.anchor.as_deref(),
                title: c.title.as_deref(),
                parent: None,
                strength: &[],
                symbol: c.symbol.as_ref().map(|s| SymbolRecord {
                    id: s.scip_id("demo", path),
                    display_name: &s.name,
                    kind: s.kind.as_str(),
                }),
            })
            .collect(),
    };
    store
        .write_split_document(source_id, &document, CHUNKER_VERSION)
        .unwrap();
    store.set_indexed_at(30).unwrap();
    let definitions = store.definition("TokenEndpoint", false).unwrap();
    assert!(definitions.ambiguous);
    assert_eq!(definitions.total, 4);
    assert_eq!(
        definitions
            .results
            .iter()
            .filter(|d| d.kind == "class")
            .count(),
        1
    );
    assert_eq!(
        definitions
            .results
            .iter()
            .filter(|d| d.kind == "method")
            .count(),
        3
    );
    let hits = store
        .search(&SearchQuery {
            text: "TokenEndpoint",
            language: None,
            path: None,
            only: None,
            limit: 10,
            allow_stale: false,
        })
        .unwrap();
    assert!(
        hits.iter()
            .any(|hit| hit.kind == "class" && hit.symbol.as_deref() == Some("TokenEndpoint")),
        "the class declaration must be in the first 10 hits; got {hits:#?}"
    );
}
