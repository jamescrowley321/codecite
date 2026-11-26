//! What a document is for, read from its path: library source, a test, an
//! example or docs. Search weights tests and examples down, so the code an
//! agent is asked to use outranks the code that exercises it, and docs a
//! little, so prose that repeats a query's words doesn't crowd out the code.
//!
//! The path conventions below are the defaults. A source's manifest can
//! replace any of them with its own globs ([`RoleGlobs`]), for a project
//! whose tests or examples live where the conventions don't look.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

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
    /// A spec: a fetched document in a spec layout.
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
        Role::classify(path, source_kind, false, &RoleRules::default())
    }

    /// The role of the document at `path` in a source of `source_kind`
    /// under `rules`: each role a source gave globs for is matched by them,
    /// and every other role by its convention. A fetched document (source
    /// kind `document`) ignores the rules: it is a spec, or docs when it is
    /// stored with its text (`has_text`).
    ///
    /// Precedence is test, then example, then docs, then source.
    pub fn classify(path: &str, source_kind: &str, has_text: bool, rules: &RoleRules) -> Role {
        if source_kind == "document" {
            return if has_text { Role::Docs } else { Role::Spec };
        }
        let segments: Vec<&str> = path.split('/').collect();
        let (file, dirs) = segments.split_last().unwrap_or((&"", &[]));
        let in_dir = |names: &[&str]| {
            dirs.iter()
                .any(|d| names.iter().any(|n| d.eq_ignore_ascii_case(n)))
        };
        // Each role's own glob set when the source gave one, else its
        // convention.
        let matches = |set: &Option<GlobSet>, convention: &dyn Fn() -> bool| {
            set.as_ref()
                .map_or_else(convention, |set| set.is_match(path))
        };
        if matches(&rules.test, &|| in_dir(&TEST_DIRS) || is_test_file(file)) {
            Role::Test
        } else if matches(&rules.example, &|| in_dir(&EXAMPLE_DIRS)) {
            Role::Example
        } else if matches(&rules.docs, &|| {
            file.rsplit_once('.')
                .is_some_and(|(_, ext)| DOCS_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
        }) {
            Role::Docs
        } else {
            Role::Source
        }
    }

    /// The role of a stored document: as [`Role::of`], except that a
    /// fetched document stored with its text is a generic page, not a spec,
    /// and so docs. A spec is stored without its text.
    pub fn of_stored(path: &str, source_kind: &str, has_text: bool) -> Role {
        Role::classify(path, source_kind, has_text, &RoleRules::default())
    }

    /// The role named `name`, as [`Role::as_str`] writes it.
    pub fn parse(name: &str) -> Option<Role> {
        match name {
            "source" => Some(Role::Source),
            "test" => Some(Role::Test),
            "example" => Some(Role::Example),
            "docs" => Some(Role::Docs),
            "spec" => Some(Role::Spec),
            _ => None,
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
    /// factor on its BM25 score. Docs' 0.8 won the sweep in
    /// `experiments/one-ranking/` (#183).
    pub fn weight(self) -> f64 {
        match self {
            Role::Source | Role::Spec => 1.0,
            Role::Docs => 0.8,
            Role::Example => 0.6,
            Role::Test => 0.4,
        }
    }
}

/// A source's own globs for the roles, as its manifest gives them. Each
/// list that is given replaces that role's convention for the source; an
/// empty list means no file has the role; a list left out keeps the
/// convention.
///
/// Globs match source-relative, `/`-separated paths as the manifest's
/// `include` globs do: `*` stays in one directory and `**` crosses them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleGlobs {
    /// Globs of test files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<Vec<String>>,
    /// Globs of example files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<Vec<String>>,
    /// Globs of prose files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<Vec<String>>,
}

impl RoleGlobs {
    /// Whether every role keeps its convention.
    pub fn is_default(&self) -> bool {
        self.test.is_none() && self.example.is_none() && self.docs.is_none()
    }
}

/// [`RoleGlobs`] compiled: for each role, its glob set, or `None` to use
/// the convention. The default keeps every convention.
#[derive(Debug, Clone, Default)]
pub struct RoleRules {
    test: Option<GlobSet>,
    example: Option<GlobSet>,
    docs: Option<GlobSet>,
}

impl RoleRules {
    /// Compiles a source's role globs.
    ///
    /// # Errors
    ///
    /// [`Error::RoleGlob`] naming the first glob that doesn't compile.
    pub fn new(globs: &RoleGlobs) -> Result<RoleRules> {
        Ok(RoleRules {
            test: compile(globs.test.as_deref())?,
            example: compile(globs.example.as_deref())?,
            docs: compile(globs.docs.as_deref())?,
        })
    }
}

/// One role's glob set, or `None` when the source left the role out.
fn compile(globs: Option<&[String]>) -> Result<Option<GlobSet>> {
    let Some(globs) = globs else {
        return Ok(None);
    };
    let bad = |pattern: &str| {
        let pattern = pattern.to_owned();
        move |source| Error::RoleGlob { pattern, source }
    };
    let mut set = GlobSetBuilder::new();
    for pattern in globs {
        set.add(
            GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map_err(bad(pattern))?,
        );
    }
    set.build().map(Some).map_err(bad(&globs.join(", ")))
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

    fn globs(test: Option<&[&str]>, example: Option<&[&str]>, docs: Option<&[&str]>) -> RoleGlobs {
        let own = |l: Option<&[&str]>| l.map(|l| l.iter().map(|g| g.to_string()).collect());
        RoleGlobs {
            test: own(test),
            example: own(example),
            docs: own(docs),
        }
    }

    fn classify(path: &str, globs: &RoleGlobs) -> Role {
        Role::classify(path, "git", true, &RoleRules::new(globs).unwrap())
    }

    #[test]
    fn default_rules_classify_as_the_conventions() {
        for path in [
            "py/src/tests/harness/rs_app.py",
            "web/client.spec.js",
            "src/main/java/ClientTest.java",
            "py/examples/descope/app.py",
            "examples/README.md",
            "py/README.md",
            "src/Latest.cs",
            "Makefile",
        ] {
            assert_eq!(
                Role::classify(path, "git", true, &RoleRules::default()),
                Role::of(path, "git"),
                "{path}"
            );
        }
    }

    #[test]
    fn a_test_list_replaces_the_test_convention() {
        let g = globs(Some(&["spec/**"]), None, None);
        assert_eq!(classify("spec/models/user_spec.rb", &g), Role::Test);
        assert_eq!(classify("tests/foo.py", &g), Role::Source);
        assert_eq!(classify("app/client_test.py", &g), Role::Source);
        // The roles left out keep their conventions.
        assert_eq!(classify("examples/app.py", &g), Role::Example);
        assert_eq!(classify("README.md", &g), Role::Docs);
    }

    #[test]
    fn an_empty_list_gives_no_file_that_role() {
        let g = globs(None, Some(&[]), None);
        assert_eq!(classify("examples/app.py", &g), Role::Source);
        assert_eq!(classify("tests/foo.py", &g), Role::Test);
    }

    #[test]
    fn a_docs_list_replaces_the_prose_extensions() {
        let g = globs(None, None, Some(&["notes/**"]));
        assert_eq!(classify("notes/plan.txt", &g), Role::Docs);
        assert_eq!(classify("README.md", &g), Role::Source);
    }

    #[test]
    fn a_star_stays_in_one_directory() {
        let g = globs(Some(&["spec/*.rb"]), None, None);
        assert_eq!(classify("spec/a.rb", &g), Role::Test);
        assert_eq!(classify("spec/models/a.rb", &g), Role::Source);
    }

    #[test]
    fn test_wins_over_example_and_example_over_docs() {
        let g = globs(Some(&["x/**"]), Some(&["x/**", "y/**"]), Some(&["**"]));
        assert_eq!(classify("x/a.md", &g), Role::Test);
        assert_eq!(classify("y/a.md", &g), Role::Example);
        assert_eq!(classify("z/a.rs", &g), Role::Docs);
    }

    #[test]
    fn a_document_source_ignores_the_rules() {
        let rules = RoleRules::new(&globs(Some(&["**"]), None, None)).unwrap();
        assert_eq!(
            Role::classify("rfc9449", "document", false, &rules),
            Role::Spec
        );
        assert_eq!(Role::classify("page", "document", true, &rules), Role::Docs);
    }

    #[test]
    fn a_bad_glob_is_an_error_naming_it() {
        let err = RoleRules::new(&globs(Some(&["a/[b"]), None, None)).expect_err("a bad glob");
        assert!(err.to_string().contains("a/[b"), "{err}");
    }

    #[test]
    fn roles_parse_from_their_names() {
        for r in [
            Role::Source,
            Role::Test,
            Role::Example,
            Role::Docs,
            Role::Spec,
        ] {
            assert_eq!(Role::parse(r.as_str()), Some(r));
        }
        assert_eq!(Role::parse("example"), Some(Role::Example));
        assert_eq!(Role::parse("Example"), None);
        assert_eq!(Role::parse(""), None);
    }

    #[test]
    fn globs_left_out_serialise_to_nothing() {
        let g = globs(Some(&[]), None, None);
        let json = serde_json::to_string(&g).unwrap();
        assert_eq!(json, r#"{"test":[]}"#);
        assert_eq!(serde_json::from_str::<RoleGlobs>(&json).unwrap(), g);
        assert_eq!(
            serde_json::from_str::<RoleGlobs>("{}").unwrap(),
            RoleGlobs::default()
        );
    }

    #[test]
    fn tests_examples_and_docs_weigh_less_than_source() {
        assert!(Role::Test.weight() < Role::Example.weight());
        assert!(Role::Example.weight() < Role::Docs.weight());
        assert!(Role::Docs.weight() < Role::Source.weight());
        assert_eq!(Role::Spec.weight(), Role::Source.weight());
    }
}
