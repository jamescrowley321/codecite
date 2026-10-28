//! The rules that govern a file: every rule of the corpus's guidance
//! documents whose scope matches the file's path, language and imports,
//! MUST before SHOULD, each cited.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use rusqlite::params;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Store;
use super::read::{instruction, pinned};
use super::search::{MAX_SEARCH_LIMIT, MAX_SNIPPET_BYTES, snippet};
use crate::{Keyword, Language, Result, imports, imports_dependency};

/// The most rule text one answer holds, over all its rules: `read`'s byte
/// budget, 64 KiB. Each rule's own text is cut as a snippet is, at 40 lines
/// and 4 KiB.
pub const MAX_CONTEXT_BYTES: usize = super::read::MAX_READ_BYTES;

/// The largest file `context` reads to find what it imports, 1 MiB, the
/// size an index run skips files over.
const MAX_INSPECTED_BYTES: u64 = 1024 * 1024;

/// What one rule applies to. Each field left empty puts no limit on it, so
/// a rule with every field empty applies to every file. A file matches
/// when it matches every field that is set, and a field when any of its
/// values matches.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuleScope {
    /// Globs matched against the file's path from its repository's root,
    /// where `*` stays inside one directory and `**` crosses them:
    /// `app/**/handlers/*.py`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Language names, as codecite detects them: `python`, `typescript`,
    /// `tsx`, `javascript`, `rust`, `go`, `csharp`, `markdown`. `.tsx` files
    /// are `tsx`, not `typescript`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// Modules the file imports, each covering the modules inside it:
    /// `requests` covers `requests.adapters`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<String>,
}

impl RuleScope {
    /// Whether the scope puts no limit on the files a rule applies to.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.languages.is_empty() && self.imports.is_empty()
    }

    /// Checks every glob and language, so a bad one stops an index run
    /// instead of matching nothing later. Returns what is wrong with the
    /// first that doesn't compile or names no language codecite detects.
    pub fn check(&self) -> std::result::Result<(), String> {
        for glob in &self.paths {
            matcher(glob).map_err(|e| format!("invalid glob {glob:?}: {e}"))?;
        }
        for module in &self.imports {
            let named = !module.is_empty()
                && module
                    .chars()
                    .all(|c| c.is_alphanumeric() || "_.-/:@".contains(c));
            if !named {
                return Err(format!(
                    "invalid import {module:?}; name a module, such as \"requests\" or \"net/http\""
                ));
            }
        }
        for language in &self.languages {
            if Language::from_name(language).is_none() {
                let names: Vec<&str> = Language::ALL.iter().map(|l| l.name()).collect();
                return Err(format!(
                    "unknown language {language:?}; use one of {}",
                    names.join(", ")
                ));
            }
        }
        Ok(())
    }

    /// Whether the scope matches the file `target` describes. A glob that
    /// doesn't compile matches nothing; an index run refuses one.
    fn matches(&self, target: &ContextTarget) -> bool {
        let path = self.paths.is_empty()
            || self
                .paths
                .iter()
                .any(|g| matcher(g).is_ok_and(|m| m.is_match(&target.path)));
        let language = self.languages.is_empty()
            || target
                .language
                .as_ref()
                .is_some_and(|l| self.languages.iter().any(|s| s == l));
        let import = self.imports.is_empty()
            || self
                .imports
                .iter()
                .any(|dep| target.imports.iter().any(|m| imports_dependency(m, dep)));
        path && language && import
    }
}

/// Compiles a scope glob as the manifest's `include` globs are compiled.
fn matcher(glob: &str) -> std::result::Result<GlobMatcher, globset::Error> {
    Ok(GlobBuilder::new(glob)
        .literal_separator(true)
        .build()?
        .compile_matcher())
}

/// The file a `context` call asks about: its path, as rule globs match it,
/// its language, and the modules it imports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextTarget {
    /// The path from the root of the repository holding the file,
    /// `/`-separated.
    pub path: String,
    /// The file's language, as codecite detects it.
    pub language: Option<String>,
    /// The modules the file imports, each once.
    pub imports: Vec<String>,
}

impl ContextTarget {
    /// Describes the file at `path`, as an agent is about to edit it.
    ///
    /// The path rule globs see is `path` from `root`, or when `root` is
    /// `None` from the nearest directory above it holding `.git`, the
    /// repository the file is in; a relative `path` starts at the current
    /// directory. A file in no repository keeps the path from the current
    /// directory, or its absolute path when it is outside it. The language
    /// comes from the extension, then the shebang. The imports are those of
    /// the file on disk, when it is there, is UTF-8 and is at most 1 MiB,
    /// together with those of `content`, the text about to be written. Only
    /// the file is read: nothing runs, and no git command.
    pub fn inspect(path: &Path, root: Option<&Path>, content: Option<&str>) -> Result<Self> {
        let cwd = std::env::current_dir()?;
        let absolute = normalize(&cwd.join(path));
        let root = match root {
            Some(root) => Some(normalize(&cwd.join(root))),
            None => repository_root(&absolute),
        };
        let relative = root
            .as_deref()
            .or(Some(&cwd))
            .and_then(|base| absolute.strip_prefix(base).ok())
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(&absolute);
        let shown: Vec<String> = relative
            .components()
            .filter_map(|c| match c {
                Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        let path = if relative.is_absolute() {
            format!("/{}", shown.join("/"))
        } else {
            shown.join("/")
        };
        let on_disk = read_capped(&absolute);
        // The text to be written is scanned to the same cap as the file.
        let content =
            content.map(|text| super::read::byte_prefix(text, MAX_INSPECTED_BYTES as usize).0);
        Ok(Self::describe(path, on_disk.as_deref(), content))
    }

    /// Describes a file at `path`, already relative to its repository, from
    /// its text on disk and the text about to be written, either of which
    /// may be missing.
    pub fn describe(path: String, on_disk: Option<&str>, content: Option<&str>) -> Self {
        let sample = on_disk.or(content).unwrap_or("");
        let language = Language::detect(&path, sample);
        let mut found: Vec<String> = Vec::new();
        if let Some(language) = language {
            for text in [on_disk, content].into_iter().flatten() {
                for module in imports(language, text) {
                    if !found.contains(&module) {
                        found.push(module);
                    }
                }
            }
        }
        ContextTarget {
            path,
            language: language.map(|l| l.name().to_owned()),
            imports: found,
        }
    }
}

/// The text of the regular file at `path` when it is UTF-8 and at most
/// [`MAX_INSPECTED_BYTES`]. The read itself stops past the cap, so a file
/// that grows after its size is checked, or a pseudo-file that reports no
/// size, can't make it read more.
fn read_capped(path: &Path) -> Option<String> {
    use std::io::Read as _;

    // Opening a FIFO blocks until a writer comes, so only a path that is a
    // regular file is opened, and the file opened must be one too.
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_INSPECTED_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_INSPECTED_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// `path` with `.` and `..` resolved by name, without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The nearest directory at or above `path`'s parent that holds `.git`
/// (a directory, or the file a worktree has).
fn repository_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .skip(1)
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

/// A `context` call: the file, and whether a stale index may answer.
#[derive(Debug, Clone, Copy)]
pub struct ContextQuery<'a> {
    /// The file the rules are for.
    pub target: &'a ContextTarget,
    /// Answer from an index older than its maximum age.
    pub allow_stale: bool,
}

/// The rules that govern one file: what `codecite context --json` prints
/// and the MCP `context` tool returns, as the same JSON.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Context {
    /// The file's path as the rules' globs saw it, from its repository's
    /// root.
    pub path: String,
    /// The file's language, when codecite knows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The modules the file imports, as the rules' `imports` saw them.
    pub imports: Vec<String>,
    /// How many rules govern the file, which may be more than `rules`
    /// holds.
    pub total: usize,
    /// The rules, MUST before SHOULD before MAY, then by document and in
    /// document order: at most 50 (`MAX_SEARCH_LIMIT`), and as many as fit
    /// in 64 KiB of text.
    pub rules: Vec<Rule>,
    /// The citations of the rules left out to stay in budget, in the same
    /// order, at most 50. `read` one for its text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub omitted: Vec<String>,
}

/// One rule that governs the file.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Rule {
    /// The rule's pinned citation: `house-rules@sha256:1a2b3c4d5e6f#section-4`.
    pub citation: String,
    /// The rule's heading, without its number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The requirement keywords in the rule, in RFC 2119's order.
    pub strength: Vec<String>,
    /// What the rule applies to; empty when it applies to every file.
    pub scope: RuleScope,
    /// How agents may use the source: `adapt`, `approach_only` or `quote`.
    pub license_class: String,
    /// When the guidance was last indexed.
    pub indexed_at: String,
    /// The rule's first 40 lines, cut to 4 KiB, as a search snippet is.
    pub text: String,
    /// Whether the text stops before the end of the rule. `read` its
    /// citation for all of it.
    pub truncated: bool,
    /// For a `quote` source: quote the rule exactly, with its citation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

/// A rule row before scope matching.
struct RuleRow {
    source_id: i64,
    number: String,
    strength: Vec<Keyword>,
    rule: Rule,
}

impl Store {
    /// The rules of the corpus's current guidance documents that govern
    /// `query.target`: each section with at least one requirement keyword
    /// whose scope matches the file. They come MUST before SHOULD before
    /// MAY (by the strongest keyword each holds: `MUST`, `MUST NOT`,
    /// `REQUIRED`, `SHALL` and `SHALL NOT`, then `SHOULD`, `SHOULD NOT`,
    /// `RECOMMENDED` and `NOT RECOMMENDED`, then `MAY` and `OPTIONAL`), then
    /// by document name and in document order. At most
    /// [`MAX_SEARCH_LIMIT`] rules and [`MAX_CONTEXT_BYTES`] of their text
    /// are returned: they stop at the first rule that doesn't fit, and it
    /// and every rule after it are cited in `omitted`.
    ///
    /// A corpus with no guidance answers with no rules. An index older than
    /// its maximum age is [`StaleIndex`](crate::Error::StaleIndex) unless `allow_stale` is set.
    pub fn context(&self, query: &ContextQuery<'_>) -> Result<Context> {
        self.check_fresh(query.allow_stale)?;
        let mut scopes = self.rule_scopes()?;
        let mut rows = self.guidance_rules()?;
        for row in &mut rows {
            if let Some(scope) = scopes.remove(&(row.source_id, row.number.clone())) {
                row.rule.scope = scope;
            }
        }
        rows.retain(|row| row.rule.scope.matches(query.target));
        // A stable sort keeps document order within each strength.
        rows.sort_by_key(|row| level(&row.strength));
        let total = rows.len();
        let limit = usize::try_from(MAX_SEARCH_LIMIT).unwrap_or(usize::MAX);
        let (mut rules, mut omitted, mut bytes) = (Vec::new(), Vec::new(), 0);
        // Once one rule doesn't fit, every later one is left out too, so a
        // short SHOULD never comes back in place of a MUST.
        let mut full = false;
        for row in rows {
            full = full || rules.len() >= limit || bytes + row.rule.text.len() > MAX_CONTEXT_BYTES;
            if !full {
                bytes += row.rule.text.len();
                rules.push(row.rule);
            } else if omitted.len() < limit {
                omitted.push(row.rule.citation);
            }
        }
        Ok(Context {
            path: query.target.path.clone(),
            language: query.target.language.clone(),
            imports: query.target.imports.clone(),
            total,
            rules,
            omitted,
        })
    }

    /// Every rule of a current guidance document that holds a requirement
    /// keyword, by document name and in document order, with no scope yet.
    fn guidance_rules(&self) -> Result<Vec<RuleRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT s.id, s.name, s.revision, s.license_class, s.indexed_at,
                    c.number, c.title, c.strength, c.text
             FROM source s
             JOIN document d ON d.source_id = s.id
             JOIN chunk c ON c.document_id = d.id
             WHERE s.guidance AND s.is_current AND c.kind = 'section'
               AND c.number IS NOT NULL AND c.strength IS NOT NULL
             ORDER BY s.name, c.id",
        )?;
        let rows = stmt.query_map([], |row| {
            let source = super::read::SourceRow {
                id: row.get(0)?,
                name: row.get(1)?,
                revision: row.get(2)?,
                license_class: row.get(3)?,
                indexed_at: row.get(4)?,
            };
            let number: String = row.get(5)?;
            let strength: String = row.get(7)?;
            let text: String = row.get(8)?;
            let (text, truncated) = snippet(&text);
            debug_assert!(text.len() <= MAX_SNIPPET_BYTES);
            let keywords: Vec<Keyword> =
                strength.split(',').filter_map(|k| k.parse().ok()).collect();
            Ok(RuleRow {
                source_id: source.id,
                rule: Rule {
                    citation: format!("{}#section-{number}", pinned(&source)),
                    title: row.get(6)?,
                    strength: keywords.iter().map(|k| k.as_str().to_owned()).collect(),
                    scope: RuleScope::default(),
                    instruction: instruction(&source.license_class, truncated),
                    license_class: source.license_class,
                    indexed_at: source.indexed_at,
                    text,
                    truncated,
                },
                number,
                strength: keywords,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every stored rule scope, by guidance source and rule number.
    fn rule_scopes(&self) -> Result<HashMap<(i64, String), RuleScope>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT source_id, number, kind, value FROM rule_scope ORDER BY rowid",
        )?;
        let mut scopes: HashMap<(i64, String), RuleScope> = HashMap::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let scope = scopes.entry((row.get(0)?, row.get(1)?)).or_default();
            let kind: String = row.get(2)?;
            let value: String = row.get(3)?;
            match kind.as_str() {
                "path" => scope.paths.push(value),
                "language" => scope.languages.push(value),
                "import" => scope.imports.push(value),
                _ => {}
            }
        }
        Ok(scopes)
    }

    /// Marks the source as guidance and replaces its rules' scopes with
    /// `scopes`, by rule number. A rule left out, or given an empty scope,
    /// applies to every file.
    pub fn set_guidance(&mut self, source_id: i64, scopes: &[(&str, &RuleScope)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE source SET guidance = 1 WHERE id = ?1", [source_id])?;
        tx.execute("DELETE FROM rule_scope WHERE source_id = ?1", [source_id])?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR IGNORE INTO rule_scope (source_id, number, kind, value)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (number, scope) in scopes {
                for (kind, values) in [
                    ("path", &scope.paths),
                    ("language", &scope.languages),
                    ("import", &scope.imports),
                ] {
                    for value in values {
                        insert.execute(params![source_id, number, kind, value])?;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
}

/// How strong a rule is, strongest first: 0 for an absolute requirement,
/// 1 for a recommendation, 2 for an option.
fn level(strength: &[Keyword]) -> u8 {
    strength
        .iter()
        .map(|k| match k {
            Keyword::Must
            | Keyword::MustNot
            | Keyword::Required
            | Keyword::Shall
            | Keyword::ShallNot => 0,
            Keyword::Should
            | Keyword::ShouldNot
            | Keyword::Recommended
            | Keyword::NotRecommended => 1,
            Keyword::May | Keyword::Optional => 2,
        })
        .min()
        .unwrap_or(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkRecord, DocumentRecord, Error, SourceRecord};

    const RULES: &str = "1a2b3c4d5e6f00000000000000000000000000000000000000000000000000ff";

    /// A section chunk numbered `number` with `text` and its keywords.
    fn rule<'a>(number: &'a str, text: &'a str, strength: &'a [Keyword]) -> ChunkRecord<'a> {
        ChunkRecord {
            kind: "section",
            start_line: 1,
            end_line: 1,
            text,
            number: Some(number),
            anchor: None,
            title: Some(text),
            parent: None,
            strength,
            symbol: None,
        }
    }

    /// A store with guidance `house-rules` and its scopes, and an ordinary
    /// spec `rfc6749` whose sections are never rules.
    fn store(scopes: &[(&str, RuleScope)]) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.db");
        let mut store = Store::open_read_write(&path).unwrap();
        for name in ["house-rules", "rfc6749"] {
            let id = store
                .upsert_source(&SourceRecord {
                    name,
                    kind: "document",
                    url: "https://example.com",
                    revision: RULES,
                    license_spdx: None,
                    license_class: "quote",
                    is_current: true,
                    obsoleted_by: None,
                    updated_by: None,
                })
                .unwrap();
            let chunks = vec![
                rule(
                    "1",
                    "Handlers SHOULD log through the audit helper.",
                    &[Keyword::Should],
                ),
                rule(
                    "2",
                    "Never read os.environ: MUST use get_settings.",
                    &[Keyword::Must],
                ),
                rule("3", "Background.", &[]),
                rule(
                    "4",
                    "Outbound calls MUST NOT use requests.",
                    &[Keyword::MustNot],
                ),
                rule("5", "Tests MAY use fixtures.", &[Keyword::May]),
                rule(
                    "6",
                    "Every file MUST carry the license header.",
                    &[Keyword::Must],
                ),
            ];
            store
                .write_document(
                    id,
                    &DocumentRecord {
                        path: name,
                        language: None,
                        content_hash: b"x",
                        size_bytes: 1,
                        dirty: false,
                        text: None,
                        chunks,
                    },
                )
                .unwrap();
            if name == "house-rules" {
                let refs: Vec<(&str, &RuleScope)> = scopes.iter().map(|(n, s)| (*n, s)).collect();
                store.set_guidance(id, &refs).unwrap();
            }
        }
        store.set_indexed_at(30).unwrap();
        drop(store);
        let store = Store::open_read_only(&path).unwrap();
        (dir, store)
    }

    fn scopes() -> Vec<(&'static str, RuleScope)> {
        vec![
            (
                "1",
                RuleScope {
                    paths: vec!["app/**/handlers/*.py".into()],
                    ..RuleScope::default()
                },
            ),
            (
                "2",
                RuleScope {
                    languages: vec!["python".into()],
                    ..RuleScope::default()
                },
            ),
            (
                "4",
                RuleScope {
                    imports: vec!["requests".into()],
                    ..RuleScope::default()
                },
            ),
            (
                "5",
                RuleScope {
                    paths: vec!["tests/**".into()],
                    languages: vec!["python".into()],
                    ..RuleScope::default()
                },
            ),
        ]
    }

    fn numbers(context: &Context) -> Vec<&str> {
        context
            .rules
            .iter()
            .map(|r| r.citation.rsplit('-').next().unwrap())
            .collect()
    }

    fn ask(store: &Store, target: ContextTarget) -> Context {
        store
            .context(&ContextQuery {
                target: &target,
                allow_stale: false,
            })
            .unwrap()
    }

    #[test]
    fn rules_match_by_glob_language_and_import_and_must_comes_first() {
        let (_dir, store) = store(&scopes());
        let handler = ContextTarget::describe(
            "app/api/handlers/partner.py".into(),
            Some("import requests.adapters\n"),
            None,
        );
        let context = ask(&store, handler);
        assert_eq!(context.language.as_deref(), Some("python"));
        assert_eq!(context.imports, ["requests.adapters"]);
        // MUST, MUST NOT and the unscoped MUST, then the SHOULD; the MAY's
        // path doesn't match and §3 holds no keyword.
        assert_eq!(numbers(&context), ["2", "4", "6", "1"]);
        assert_eq!(context.total, 4);
        assert!(context.omitted.is_empty());
        let first = &context.rules[0];
        assert_eq!(first.citation, "house-rules@sha256:1a2b3c4d5e6f#section-2");
        assert_eq!(first.strength, ["MUST"]);
        assert_eq!(first.scope.languages, ["python"]);
        assert_eq!(first.license_class, "quote");
        assert!(first.instruction.is_some());
        assert!(context.rules[2].scope.is_empty());
    }

    #[test]
    fn a_rule_with_no_scope_applies_everywhere() {
        let (_dir, store) = store(&scopes());
        let readme = ContextTarget::describe("README.md".into(), None, None);
        assert_eq!(numbers(&ask(&store, readme)), ["6"]);
        let unknown = ContextTarget::describe("Makefile".into(), None, None);
        assert_eq!(numbers(&ask(&store, unknown)), ["6"]);
    }

    #[test]
    fn every_set_field_must_match() {
        let (_dir, store) = store(&scopes());
        // §5 needs both a tests/ path and Python.
        let test = ContextTarget::describe("tests/test_x.py".into(), None, None);
        assert_eq!(numbers(&ask(&store, test)), ["2", "6", "5"]);
        let ts = ContextTarget::describe("tests/x.ts".into(), None, None);
        assert_eq!(numbers(&ask(&store, ts)), ["6"]);
    }

    #[test]
    fn a_glob_star_stays_in_one_directory() {
        let (_dir, store) = store(&scopes());
        let deep = ContextTarget::describe("app/api/handlers/v2/partner.py".into(), None, None);
        assert_eq!(numbers(&ask(&store, deep)), ["2", "6"]);
        let direct = ContextTarget::describe("app/handlers/partner.py".into(), None, None);
        assert_eq!(numbers(&ask(&store, direct)), ["2", "6", "1"]);
    }

    #[test]
    fn imports_come_from_the_file_and_the_text_to_be_written() {
        let (_dir, store) = store(&scopes());
        let target = ContextTarget::describe(
            "svc/client.py".into(),
            Some("import os\n"),
            Some("import os\nimport requests\n"),
        );
        assert_eq!(target.imports, ["os", "requests"]);
        assert_eq!(numbers(&ask(&store, target)), ["2", "4", "6"]);
        // `requests_oauthlib` is another module.
        let other =
            ContextTarget::describe("x.py".into(), None, Some("import requests_oauthlib\n"));
        assert_eq!(numbers(&ask(&store, other)), ["2", "6"]);
    }

    #[test]
    fn rules_past_the_byte_budget_are_cited_in_omitted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.db");
        let mut store = Store::open_read_write(&path).unwrap();
        let id = store
            .upsert_source(&SourceRecord {
                name: "big",
                kind: "document",
                url: "u",
                revision: RULES,
                license_spdx: None,
                license_class: "adapt",
                is_current: true,
                obsoleted_by: None,
                updated_by: None,
            })
            .unwrap();
        // 20 MUSTs of about 4 KiB each, of which 16 fit in 64 KiB, then a
        // short SHOULD that would fit in what is left.
        let text = format!("You MUST {}", "x".repeat(4000));
        let numbers: Vec<String> = (1..=20).map(|n| n.to_string()).collect();
        let mut chunks: Vec<ChunkRecord<'_>> = numbers
            .iter()
            .map(|n| rule(n, &text, &[Keyword::Must]))
            .collect();
        chunks.push(rule("21", "You SHOULD.", &[Keyword::Should]));
        store
            .write_document(
                id,
                &DocumentRecord {
                    path: "big",
                    language: None,
                    content_hash: b"x",
                    size_bytes: 1,
                    dirty: false,
                    text: None,
                    chunks,
                },
            )
            .unwrap();
        store.set_guidance(id, &[]).unwrap();
        store.set_indexed_at(30).unwrap();
        let context = ask(&store, ContextTarget::describe("a.rs".into(), None, None));
        assert_eq!(context.total, 21);
        assert_eq!(context.rules.len(), 16);
        let bytes: usize = context.rules.iter().map(|r| r.text.len()).sum();
        assert!(bytes <= MAX_CONTEXT_BYTES, "{bytes}");
        assert_eq!(context.omitted.len(), 5);
        assert_eq!(context.omitted[0], "big@sha256:1a2b3c4d5e6f#section-17");
        assert_eq!(context.omitted[4], "big@sha256:1a2b3c4d5e6f#section-21");
        assert!(context.rules[0].instruction.is_none());
    }

    #[test]
    fn a_source_upserted_again_is_not_guidance_until_marked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.db");
        let mut store = Store::open_read_write(&path).unwrap();
        let record = SourceRecord {
            name: "house-rules",
            kind: "document",
            url: "u",
            revision: RULES,
            license_spdx: None,
            license_class: "quote",
            is_current: true,
            obsoleted_by: None,
            updated_by: None,
        };
        let id = store.upsert_source(&record).unwrap();
        let chunks = vec![rule("1", "You MUST.", &[Keyword::Must])];
        store
            .write_document(
                id,
                &DocumentRecord {
                    path: "house-rules",
                    language: None,
                    content_hash: b"x",
                    size_bytes: 1,
                    dirty: false,
                    text: None,
                    chunks,
                },
            )
            .unwrap();
        let scope = RuleScope {
            languages: vec!["python".into()],
            ..RuleScope::default()
        };
        store.set_guidance(id, &[("1", &scope)]).unwrap();
        store.set_indexed_at(30).unwrap();
        let target = ContextTarget::describe("a.py".into(), None, None);
        assert_eq!(ask(&store, target.clone()).total, 1);

        // The manifest now lists it as a plain document, same bytes.
        assert_eq!(store.upsert_source(&record).unwrap(), id);
        assert_eq!(ask(&store, target).total, 0);
        let scopes: i64 = store
            .conn
            .query_row("SELECT count(*) FROM rule_scope", [], |row| row.get(0))
            .unwrap();
        assert_eq!(scopes, 0);
    }

    #[test]
    fn a_corpus_without_guidance_has_no_rules() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.db");
        let mut store = Store::open_read_write(&path).unwrap();
        store.set_indexed_at(30).unwrap();
        let context = ask(&store, ContextTarget::describe("a.py".into(), None, None));
        assert_eq!(context.total, 0);
        assert!(context.rules.is_empty());
    }

    #[test]
    fn a_stale_index_is_refused_unless_allowed() {
        let (dir, store) = store(&scopes());
        rusqlite::Connection::open(dir.path().join("demo.db"))
            .unwrap()
            .execute(
                "UPDATE meta SET value = '2026-01-01T00:00:00Z' WHERE key = 'indexed_at'",
                [],
            )
            .unwrap();
        let target = ContextTarget::describe("a.py".into(), None, None);
        let query = |allow_stale| ContextQuery {
            target: &target,
            allow_stale,
        };
        let err = store.context(&query(false)).unwrap_err();
        assert!(matches!(err, Error::StaleIndex { .. }), "{err:?}");
        assert_eq!(store.context(&query(true)).unwrap().total, 2);
    }

    #[test]
    fn inspect_finds_the_repository_root_and_reads_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("app/handlers")).unwrap();
        let file = repo.join("app/handlers/x.py");
        std::fs::write(&file, "from httpx import Client\n").unwrap();

        let target = ContextTarget::inspect(&file, None, Some("import requests\n")).unwrap();
        assert_eq!(target.path, "app/handlers/x.py");
        assert_eq!(target.language.as_deref(), Some("python"));
        assert_eq!(target.imports, ["httpx", "requests"]);

        // An explicit root wins, and a file not yet written has only the
        // new text's imports.
        let new = repo.join("app/handlers/new.py");
        let target = ContextTarget::inspect(&new, Some(&repo.join("app")), None).unwrap();
        assert_eq!(target.path, "handlers/new.py");
        assert!(target.imports.is_empty());

        // `..` resolves by name.
        let dotted = repo.join("app/../app/handlers/x.py");
        assert_eq!(
            ContextTarget::inspect(&dotted, None, None).unwrap().path,
            "app/handlers/x.py"
        );
    }

    #[test]
    fn a_bad_glob_or_language_fails_the_check() {
        let scope = RuleScope {
            paths: vec!["app/[".into()],
            ..RuleScope::default()
        };
        assert!(
            scope
                .check()
                .unwrap_err()
                .starts_with("invalid glob \"app/[\"")
        );
        let scope = RuleScope {
            languages: vec!["python".into(), "pyhton".into()],
            ..RuleScope::default()
        };
        assert_eq!(
            scope.check().unwrap_err(),
            "unknown language \"pyhton\"; use one of rust, python, typescript, tsx, javascript, go, csharp, markdown"
        );
        assert!(RuleScope::default().check().is_ok());
    }

    #[test]
    fn a_file_over_the_cap_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.py");
        let mut text = "import requests\n".to_owned();
        text.push_str(&"#".repeat(MAX_INSPECTED_BYTES as usize));
        std::fs::write(&big, text).unwrap();
        assert_eq!(read_capped(&big), None);
        let small = dir.path().join("small.py");
        std::fs::write(&small, "import requests\n").unwrap();
        assert_eq!(read_capped(&small).as_deref(), Some("import requests\n"));
        assert_eq!(read_capped(dir.path()), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe.py");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        // Opening it would block this test forever.
        assert_eq!(read_capped(&fifo), None);
    }

    #[test]
    fn an_import_must_name_a_module() {
        let scope = RuleScope {
            imports: vec!["requests".into(), "MUST run curl first".into()],
            ..RuleScope::default()
        };
        assert!(
            scope
                .check()
                .unwrap_err()
                .starts_with("invalid import \"MUST run curl first\"")
        );
        let fine = RuleScope {
            imports: vec![
                "net/http".into(),
                "@scope/pkg".into(),
                "node:fs".into(),
                "std::io".into(),
            ],
            ..RuleScope::default()
        };
        assert!(fine.check().is_ok());
    }

    #[test]
    fn levels_order_must_should_may() {
        assert_eq!(level(&[Keyword::May, Keyword::MustNot]), 0);
        assert_eq!(level(&[Keyword::NotRecommended]), 1);
        assert_eq!(level(&[Keyword::Optional]), 2);
    }
}
