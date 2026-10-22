//! What a document is for, read from its path: library source, a test, an
//! example or docs. Search weights tests and examples down, so the code an
//! agent is asked to use outranks the code that exercises it.

/// What a document is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Library or application code.
    Source,
    /// A test, test helper or test fixture.
    Test,
    /// An example or sample app.
    Example,
    /// Prose: Markdown, reStructuredText or plain text.
    Docs,
    /// A spec document source.
    Spec,
}

/// Directories whose files are tests, compared case-insensitively. A
/// conformance suite tests the code it drives.
const TEST_DIRS: [&str; 5] = ["test", "tests", "__tests__", "testdata", "conformance"];

/// Directories whose files are examples, compared case-insensitively.
const EXAMPLE_DIRS: [&str; 6] = ["example", "examples", "sample", "samples", "demo", "demos"];

/// Extensions of prose files.
const DOCS_EXTENSIONS: [&str; 5] = ["md", "markdown", "rst", "txt", "adoc"];

impl Role {
    /// The role of the document at `path` in a source of `source_kind`
    /// (`local`, `git` or `document`).
    ///
    /// A test directory or test file name wins over an example directory,
    /// and both over a prose extension, so `examples/README.md` is an
    /// example and `tests/fixtures/notes.md` a test.
    pub fn of(path: &str, source_kind: &str) -> Role {
        if source_kind == "document" {
            return Role::Spec;
        }
        let segments: Vec<&str> = path.split('/').collect();
        let (file, dirs) = segments.split_last().unwrap_or((&"", &[]));
        let in_dir = |names: &[&str]| {
            dirs.iter()
                .any(|d| names.iter().any(|n| d.eq_ignore_ascii_case(n)))
        };
        if in_dir(&TEST_DIRS) || is_test_file(file) {
            Role::Test
        } else if in_dir(&EXAMPLE_DIRS) {
            Role::Example
        } else if file
            .rsplit_once('.')
            .is_some_and(|(_, ext)| DOCS_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
        {
            Role::Docs
        } else {
            Role::Source
        }
    }

    /// The value search results carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Source => "source",
            Role::Test => "test",
            Role::Example => "example",
            Role::Docs => "docs",
            Role::Spec => "spec",
        }
    }

    /// How much a match in a document of this role counts in search, as a
    /// factor on its BM25 score.
    pub fn weight(self) -> f64 {
        match self {
            Role::Source | Role::Docs | Role::Spec => 1.0,
            Role::Example => 0.6,
            Role::Test => 0.4,
        }
    }
}

/// Whether a file name is a test by convention: `test_x.py`, `x_test.py`,
/// `x_test.go`, `x.test.ts`, `x.spec.js`, `conftest.py`, `XTest.java` or
/// `XTests.cs`.
fn is_test_file(file: &str) -> bool {
    let (stem, ext) = file.rsplit_once('.').unwrap_or((file, ""));
    let lower = stem.to_ascii_lowercase();
    (ext == "py" && (lower.starts_with("test_") || lower == "conftest"))
        || lower.ends_with("_test")
        || lower.ends_with(".test")
        || lower.ends_with(".spec")
        // The capital keeps `Latest.cs` and `Contest.java` out.
        || stem.ends_with("Test")
        || stem.ends_with("Tests")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(path: &str) -> &'static str {
        Role::of(path, "git").as_str()
    }

    #[test]
    fn test_directories_and_file_names_are_tests() {
        for path in [
            "py/packages/fastapi-identity-model/tests/test_middleware.py",
            "py/src/tests/harness/rs_app.py",
            "src/__tests__/client.ts",
            "go/testdata/token.json",
            "app/test_client.py",
            "app/client_test.py",
            "conftest.py",
            "pkg/client_test.go",
            "web/client.test.ts",
            "web/client.spec.js",
            "src/ClientTests.cs",
            "src/main/java/ClientTest.java",
            "Tests/Client.cs",
            "conformance/app_fastapi.py",
        ] {
            assert_eq!(role(path), "test", "{path}");
        }
    }

    #[test]
    fn example_directories_are_examples() {
        for path in [
            "py/examples/descope/app.py",
            "examples/README.md",
            "samples/Web/Program.cs",
            "demo/main.go",
        ] {
            assert_eq!(role(path), "example", "{path}");
        }
    }

    #[test]
    fn prose_is_docs_and_everything_else_source() {
        assert_eq!(role("py/README.md"), "docs");
        assert_eq!(role("docs/guide.rst"), "docs");
        assert_eq!(role("CHANGELOG.txt"), "docs");
        for path in [
            "py/packages/fastapi-identity-model/fastapi_identity_model/middleware.py",
            "src/latest.rs",
            "src/Latest.cs",
            "src/contest.py",
            "src/attestation.go",
            "testing/helpers.go",
            "Makefile",
        ] {
            assert_eq!(role(path), "source", "{path}");
        }
    }

    #[test]
    fn a_document_source_is_a_spec() {
        assert_eq!(Role::of("rfc9449", "document"), Role::Spec);
    }

    #[test]
    fn tests_and_examples_weigh_less_than_source() {
        assert!(Role::Test.weight() < Role::Example.weight());
        assert!(Role::Example.weight() < Role::Source.weight());
        assert_eq!(Role::Docs.weight(), Role::Source.weight());
    }
}
