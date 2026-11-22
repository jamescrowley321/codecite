//! C documentation belongs with the definition it describes.
use codecite_index::Chunker;

#[test]
fn c_function_chunk_includes_its_immediately_preceding_doc_comment() {
    let source =
        "/* Initiate an outgoing connection. */\nint connect_socket(void) {\n    return 0;\n}\n";
    let chunks = Chunker::new().unwrap().chunk("socket.c", source);
    let function = chunks
        .iter()
        .find(|chunk| {
            chunk
                .symbol
                .as_ref()
                .is_some_and(|symbol| symbol.name == "connect_socket")
        })
        .expect("the C function must be indexed");
    assert_eq!(
        function.start_line, 1,
        "the documentation must be searchable with the function"
    );
    assert!(
        function
            .text
            .starts_with("/* Initiate an outgoing connection. */")
    );
}

#[test]
fn separated_or_trailing_c_comments_do_not_attach_to_the_next_function() {
    for (source, expected_start) in [
        ("/* a module note */\n\nint run(void) { return 0; }\n", 3),
        (
            "int value; // describes the value\nint run(void) { return 0; }\n",
            2,
        ),
    ] {
        let chunks = Chunker::new().unwrap().chunk("input.c", source);
        let function = chunks
            .iter()
            .find(|chunk| {
                chunk
                    .symbol
                    .as_ref()
                    .is_some_and(|symbol| symbol.name == "run")
            })
            .expect("the C function must be indexed");
        assert_eq!(function.start_line, expected_start);
    }
}
