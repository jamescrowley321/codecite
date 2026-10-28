//! Diffing two pinned versions of a spec document: the normative
//! statements one has that the other lacks, or has at another strength.
//!
//! See `docs/superpowers/specs/2026-10-28-spec-version-diff-design.md`. The
//! statement extraction is [`version_units`], kept in one function so that
//! it can be unified with other normative-statement extraction later.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::params;
use schemars::JsonSchema;
use serde::Serialize;

use super::Store;
use super::read::{
    MAX_READ_BYTES, QUOTE_INSTRUCTION, QUOTE_TRUNCATED_INSTRUCTION, SourceRow, byte_prefix, pinned,
};
use crate::requirement_keywords;
use crate::{Error, Keyword, MAX_SEARCH_LIMIT, MAX_SNIPPET_BYTES, Result, SectionNumber};

/// What to diff.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffOptions<'a> {
    /// Only this section, by number: `2.4`. `None` diffs every section.
    pub section: Option<&'a str>,
    /// The older version, by hash (`sha256:023434…` or `023434…`). `None`
    /// is the older version pinned last.
    pub from: Option<&'a str>,
    /// The newer version, by hash. `None` is the current version.
    pub to: Option<&'a str>,
    /// Diff an index older than its maximum age instead of refusing it.
    pub allow_stale: bool,
}

/// How two pinned versions of a document differ in their rules, in the
/// shape the CLI prints as JSON and the MCP server returns.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DiffResult {
    /// The document's name: `openid-connect-backchannel-1_0`.
    pub document: String,
    /// The older version, pinned: `doc@sha256:<12 hex>`.
    pub from: String,
    /// The newer version, pinned.
    pub to: String,
    /// The section diffed, when one was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// How agents may use the document: `quote` for the specs.
    pub license_class: String,
    /// When the newer version was last indexed.
    pub indexed_at: String,
    /// The normative statements added, removed or changed, in the newer
    /// version's order, at most 50.
    pub changes: Vec<VersionChange>,
    /// Normative statements whose wording changed but not their content:
    /// a date in a reference, a spelling fix. At most 50.
    pub reworded: Vec<VersionChange>,
    /// How many changes there are, listed or not.
    pub total: usize,
    /// How many rewordings there are, listed or not.
    pub reworded_total: usize,
    /// Whether `changes` or `reworded` stops short of all of them: each
    /// lists at most 50, and their texts together hold at most 64 KiB.
    pub truncated: bool,
    /// What the license asks of the agent: a `quote` source's text is to be
    /// quoted exactly, with its citation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

impl fmt::Display for DiffResult {
    /// The human form: the versions compared, then each change with its
    /// citations and texts, `-` for the older and `+` for the newer.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} → {}", self.from, self.to)?;
        if let Some(section) = &self.section {
            write!(f, " · section {section}")?;
        }
        writeln!(
            f,
            "\n{} normative changes · {} · indexed {}",
            self.total, self.license_class, self.indexed_at
        )?;
        for (n, c) in self.changes.iter().enumerate() {
            writeln!(f)?;
            write_change(f, n + 1, c)?;
        }
        if self.changes.len() < self.total {
            writeln!(
                f,
                "\n… ({} more not shown)",
                self.total - self.changes.len()
            )?;
        }
        if self.reworded_total > 0 {
            writeln!(
                f,
                "\n{} reworded, with no normative change:",
                self.reworded_total
            )?;
            for c in &self.reworded {
                let first = c.to_text.as_deref().and_then(|t| t.lines().next());
                let first: String = first.unwrap_or_default().trim().chars().take(80).collect();
                writeln!(f, "  {} · {first}", c.to.as_deref().unwrap_or("-"))?;
            }
            if self.reworded.len() < self.reworded_total {
                writeln!(
                    f,
                    "  … ({} more not shown)",
                    self.reworded_total - self.reworded.len()
                )?;
            }
        }
        Ok(())
    }
}

/// Writes one change: its number, kind and strengths, its citations, and
/// its texts line by line.
fn write_change(f: &mut fmt::Formatter<'_>, n: usize, c: &VersionChange) -> fmt::Result {
    let joined = |s: &Option<Vec<String>>| match s.as_deref() {
        Some([]) => "no keyword".to_owned(),
        Some(k) => k.join(", "),
        None => String::new(),
    };
    write!(f, "{n}. {} · section {}", c.change, c.section)?;
    match (&c.from_strength, &c.to_strength) {
        (Some(_), Some(_)) if c.change == "strength_changed" => write!(
            f,
            " · {} → {}",
            joined(&c.from_strength),
            joined(&c.to_strength)
        )?,
        (_, Some(_)) => write!(f, " · {}", joined(&c.to_strength))?,
        (Some(_), None) => write!(f, " · {}", joined(&c.from_strength))?,
        (None, None) => {}
    }
    writeln!(f)?;
    if let Some(from) = &c.from {
        writeln!(f, "   from {from}")?;
    }
    if let Some(to) = &c.to {
        writeln!(f, "   to   {to}")?;
    }
    for line in c.from_text.iter().flat_map(|t| t.lines()) {
        writeln!(f, "   - {line}")?;
    }
    for line in c.to_text.iter().flat_map(|t| t.lines()) {
        writeln!(f, "   + {line}")?;
    }
    Ok(())
}

/// One normative statement that differs between the versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VersionChange {
    /// `added`, `removed`, `changed`, `strength_changed` or `reworded`.
    pub change: String,
    /// The section's number in the newer version, or the older one's for a
    /// section the newer version doesn't have.
    pub section: String,
    /// Where the statement is in the older version, or for an added one,
    /// the section it is missing from. Left out when the older version has
    /// no such section.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Where the statement is in the newer version, or for a removed one,
    /// the section it is missing from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// The statement's exact text in the older version, at most 4 KiB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_text: Option<String>,
    /// The statement's exact text in the newer version, at most 4 KiB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_text: Option<String>,
    /// The requirement keywords of the older statement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_strength: Option<Vec<String>>,
    /// The requirement keywords of the newer statement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_strength: Option<Vec<String>>,
    /// Whether `from_text` or `to_text` was cut at 4 KiB; `read` the
    /// citation for the whole statement.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub text_truncated: bool,
}

/// One statement of a section: a step, a definition with its term, a
/// paragraph, or a block of preformatted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unit {
    /// The exact text, lines joined with `\n`.
    pub(crate) text: String,
    /// The step's number, for a step.
    pub(crate) step: Option<String>,
    /// A definition's term, `exp`, when the unit starts with one.
    pub(crate) term: Option<String>,
    /// Its requirement keywords.
    pub(crate) keywords: Vec<Keyword>,
    /// Its content words: lower-cased, without punctuation, month names or
    /// four-digit years.
    pub(crate) key: Vec<String>,
}

impl Unit {
    /// A rule: it holds a requirement keyword, or it is a step.
    fn is_normative(&self) -> bool {
        self.step.is_some() || !self.keywords.is_empty()
    }
}

/// A step of a section, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StepText {
    pub(crate) number: String,
    pub(crate) start_line: u32,
    pub(crate) end_line: u32,
    pub(crate) text: String,
}

/// Splits a section's text into the units [`Store::diff`] compares.
///
/// `text` is the section's stored text, whose first line, the heading, is
/// line `start_line` of the document, and `steps` are its steps. Each step
/// is one unit and its lines belong to no other. Of the other lines, a run
/// of lines that start with whitespace, up to a blank line, is one unit
/// (an older RFC's wrapped plain text, or a `<pre>` block); a definition's
/// term, a short line with no sentence punctuation followed by a line,
/// joins that line; and any other line is a unit of its own.
pub(crate) fn version_units(text: &str, start_line: u32, steps: &[StepText]) -> Vec<Unit> {
    let lines: Vec<&str> = text.lines().collect();
    let mut in_step: Vec<Option<usize>> = vec![None; lines.len()];
    for (k, step) in steps.iter().enumerate() {
        for line in step.start_line..=step.end_line {
            if let Some(slot) = line
                .checked_sub(start_line)
                .and_then(|i| in_step.get_mut(i as usize))
            {
                *slot = Some(k);
            }
        }
    }
    let mut emitted = vec![false; steps.len()];
    let mut units = Vec::new();
    let free = |j: usize| j < lines.len() && in_step[j].is_none() && !lines[j].trim().is_empty();
    let indented = |line: &str| line.starts_with(char::is_whitespace);
    // Line 0 is the heading.
    let mut i = 1;
    while i < lines.len() {
        if let Some(k) = in_step[i] {
            if !emitted[k] {
                emitted[k] = true;
                units.push(step_unit(&steps[k]));
            }
            i += 1;
            continue;
        }
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
        } else if indented(line) {
            let mut j = i;
            while free(j) && indented(lines[j]) {
                j += 1;
            }
            let block = &lines[i..j];
            let term = (block.len() > 1 && is_term(block[0])).then(|| block[0].trim());
            units.push(new_unit(block.join("\n"), None, term));
            i = j;
        } else if is_term(line) && free(i + 1) && !indented(lines[i + 1]) {
            units.push(new_unit(
                format!("{line}\n{}", lines[i + 1]),
                None,
                Some(line.trim()),
            ));
            i += 2;
        } else {
            units.push(new_unit(line.to_owned(), None, None));
            i += 1;
        }
    }
    // A step whose lines fall outside its section's text still counts.
    for (k, step) in steps.iter().enumerate() {
        if !emitted[k] {
            units.push(step_unit(step));
        }
    }
    units
}

fn step_unit(step: &StepText) -> Unit {
    new_unit(step.text.clone(), Some(step.number.clone()), None)
}

fn new_unit(text: String, step: Option<String>, term: Option<&str>) -> Unit {
    Unit {
        keywords: requirement_keywords(&text),
        key: content_key(&text),
        term: term.map(str::to_owned),
        step,
        text,
    }
}

/// A definition's term: a short line, at most four words, with a letter
/// or digit, no sentence punctuation at its end and no requirement keyword.
fn is_term(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty()
        && t.split_whitespace().count() <= 4
        && t.chars().any(char::is_alphanumeric)
        && !t.ends_with(['.', ':', ';', '?', '!', ',', ')'])
        && requirement_keywords(t).is_empty()
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// A unit's content words: runs of letters, digits and `_`, lower-cased,
/// without what changes between versions with no change to a rule.
///
/// An OpenID page renders each reference's tooltip inline, so a citation
/// reads `[RFC6749] (Hardt, D., Ed., “The OAuth 2.0 …,” October 2012.)`:
/// the parenthesis with a quoted title is dropped and the label kept. A
/// cross-reference reads `Section 7.1 (TLS Requirements)`: its number is
/// dropped and its title kept, so a renumbering is no change. Month names
/// and four-digit years go too, which a date is made of; `MAY` lower-cases
/// to the month `may`, so only a word that is not all uppercase is dropped
/// as a month.
fn content_key(text: &str) -> Vec<String> {
    static ANNOTATION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\([^()“]*“[^”]*”[^()]*\d{4}\.?\)|Section\s+[0-9A-Z][0-9.]*\s*(?:\()")
            .expect("a valid regex")
    });
    let text = ANNOTATION.replace_all(text, |c: &regex::Captures<'_>| {
        // A cross-reference keeps its parenthesis, whose title follows.
        if c[0].starts_with('(') { " " } else { " (" }
    });
    let words: Vec<&str> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    let month = |w: &str| MONTHS.contains(&w.to_lowercase().as_str()) && w != w.to_uppercase();
    let year = |w: &str| {
        w.len() == 4
            && w.chars().all(|c| c.is_ascii_digit())
            && (w.starts_with("19") || w.starts_with("20"))
    };
    let mut key = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        // Only a month followed by a year is a date; a number alone, like
        // 2048 bits, is content.
        if month(words[i]) && words.get(i + 1).is_some_and(|w| year(w)) {
            i += 2;
            continue;
        }
        key.push(words[i].to_lowercase());
        i += 1;
    }
    key
}

/// How a pair of units, one per version, differs, if it is reported: the
/// `change` of a [`VersionChange`].
fn classify(old: &Unit, new: &Unit) -> Option<&'static str> {
    if old.text == new.text || !(old.is_normative() || new.is_normative()) {
        None
    } else if old.keywords != new.keywords {
        Some("strength_changed")
    } else if old.key == new.key || spelling_only(&old.key, &new.key) {
        Some("reworded")
    } else {
        Some("changed")
    }
}

/// Whether the words that differ between two keys pair off one to one as
/// spelling fixes: both at least five letters long, one edit apart, and
/// neither the other with a letter added in front or behind. Short words
/// are identifiers (`sub`, `sid`) and never count, and two edits already
/// reach opposites (`signed`, `unsigned`; `include`, `exclude`). Keys that
/// differ only in order are not spelling fixes: moving a word moves a rule.
fn spelling_only(old: &[String], new: &[String]) -> bool {
    let (mut removed, mut added) = (counts(old), counts(new));
    for (word, n) in counts(new) {
        if let Some(m) = removed.get_mut(word) {
            let common = (*m).min(n);
            *m -= common;
            *added.get_mut(word).expect("counted") -= common;
        }
    }
    let (removed, mut added) = (flatten(removed), flatten(added));
    if removed.is_empty() || removed.len() != added.len() {
        return false;
    }
    removed.iter().all(|r| {
        let found = added.iter().position(|a| {
            r.chars().count() >= 5
                && a.chars().count() >= 5
                && edit_distance(r, a) == 1
                && !(a.ends_with(r) || r.ends_with(a) || a.starts_with(r) || r.starts_with(a))
        });
        found.map(|i| added.swap_remove(i)).is_some()
    })
}

/// The words counted in `c`, each as many times as counted, sorted.
fn flatten(c: HashMap<&str, usize>) -> Vec<&str> {
    let mut words: Vec<&str> = c
        .into_iter()
        .flat_map(|(w, n)| std::iter::repeat_n(w, n))
        .collect();
    words.sort_unstable();
    words
}

fn counts(words: &[String]) -> HashMap<&str, usize> {
    let mut c = HashMap::new();
    for w in words {
        *c.entry(w.as_str()).or_insert(0) += 1;
    }
    c
}

/// The Levenshtein distance between two words, by character.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// The Dice coefficient of two keys' word multisets: from 0, nothing in
/// common, to 1, the same words.
fn dice(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let mut ca = counts(a);
    let mut common = 0usize;
    for w in b {
        if let Some(n) = ca.get_mut(w.as_str())
            && *n > 0
        {
            *n -= 1;
            common += 1;
        }
    }
    #[expect(clippy::cast_precision_loss, reason = "word counts are small")]
    let score = 2.0 * common as f64 / (a.len() + b.len()) as f64;
    score
}

/// The most unpaired (old, new) unit pairs one section scores by
/// similarity: a section of 500 statements in each version.
const MAX_SIMILARITY_PAIRS: usize = 250_000;

/// The least [`dice`] score at which two units pair.
const MIN_SIMILARITY: f64 = 0.5;

/// Whether two units may pair at all: steps pair with steps, and two
/// definitions only when their terms are the same.
fn may_pair(a: &Unit, b: &Unit) -> bool {
    a.step.is_some() == b.step.is_some()
        && match (&a.term, &b.term) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        }
}

/// Pairs the units of two versions: the same content first, in order, then
/// the most similar. Returns the pairs, by new index, then the unpaired old
/// and new indexes.
fn pair(old: &[Unit], new: &[Unit]) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    let mut old_used = vec![false; old.len()];
    let mut new_used = vec![false; new.len()];
    let mut pairs = Vec::new();
    let mut by_key: HashMap<&[String], Vec<usize>> = HashMap::new();
    for (i, o) in old.iter().enumerate() {
        by_key.entry(o.key.as_slice()).or_default().push(i);
    }
    for (j, n) in new.iter().enumerate() {
        let same = by_key.get(n.key.as_slice()).map_or(&[][..], Vec::as_slice);
        if let Some(&i) = same.iter().find(|&&i| !old_used[i] && may_pair(&old[i], n)) {
            old_used[i] = true;
            new_used[j] = true;
            pairs.push((i, j));
        }
    }
    let mut candidates = Vec::new();
    let left = |used: &[bool]| used.iter().filter(|u| !**u).count();
    // Scoring every pair is quadratic; past a bound, what is left unpaired
    // is reported as added and removed rather than paired by similarity.
    let bounded = left(&old_used).saturating_mul(left(&new_used)) <= MAX_SIMILARITY_PAIRS;
    for (i, o) in old
        .iter()
        .enumerate()
        .filter(|(i, _)| bounded && !old_used[*i])
    {
        for (j, n) in new.iter().enumerate().filter(|(j, _)| !new_used[*j]) {
            let score = dice(&o.key, &n.key);
            if may_pair(o, n) && score >= MIN_SIMILARITY {
                candidates.push((score, i, j));
            }
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then((a.2, a.1).cmp(&(b.2, b.1))));
    for (_, i, j) in candidates {
        if !old_used[i] && !new_used[j] {
            old_used[i] = true;
            new_used[j] = true;
            pairs.push((i, j));
        }
    }
    pairs.sort_by_key(|&(_, j)| j);
    let unused = |used: &[bool]| (0..used.len()).filter(|&i| !used[i]).collect();
    (pairs, unused(&old_used), unused(&new_used))
}

/// A section of one version, as `diff` needs it.
struct Section {
    number: String,
    anchor: Option<String>,
    start_line: u32,
    text: String,
    steps: Vec<StepText>,
}

/// A section number as asked for, without `§` or `section-`: `2.4`.
fn section_asked(section: &str) -> String {
    let mut s = section.trim();
    for prefix in ["§", "section-", "Section", "section"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim_start();
        }
    }
    s.trim_end_matches('.').to_owned()
}

fn strength(keywords: &[Keyword]) -> Vec<String> {
    keywords.iter().map(|k| k.as_str().to_owned()).collect()
}

/// The most bytes of statement text one diff returns, over all its
/// changes and rewordings: `read`'s budget, so one call can't flood an
/// agent's context.
const MAX_DIFF_BYTES: usize = MAX_READ_BYTES;

/// Cuts a change's texts to [`MAX_SNIPPET_BYTES`] each, setting
/// `text_truncated` when either is cut, and returns their size.
fn cap_texts(c: &mut VersionChange) -> usize {
    let mut size = 0;
    for text in [&mut c.from_text, &mut c.to_text].into_iter().flatten() {
        let (cut, over) = byte_prefix(text, MAX_SNIPPET_BYTES);
        let cut = cut.to_owned();
        c.text_truncated |= over;
        size += cut.len();
        *text = cut;
    }
    size
}

/// Keeps the entries of `list` that fit in the limits: at most
/// [`MAX_SEARCH_LIMIT`] entries, and texts within `budget` bytes, which it
/// spends. Returns whether any were dropped.
fn keep_within(list: &mut Vec<VersionChange>, budget: &mut usize) -> bool {
    let max = MAX_SEARCH_LIMIT as usize;
    let mut kept = 0;
    for c in list.iter_mut() {
        let size = cap_texts(c);
        if kept == max || size > *budget {
            break;
        }
        *budget -= size;
        kept += 1;
    }
    let dropped = kept < list.len();
    list.truncate(kept);
    dropped
}

impl Store {
    /// Diffs two pinned versions of the document `document`, a name or an
    /// alias: the normative statements added, removed, changed or changed
    /// in strength, with both versions' citations, and apart from them the
    /// ones only reworded.
    ///
    /// The versions default to the current one and the older one pinned
    /// last. A document the corpus doesn't have is
    /// [`Error::UnknownSource`]; a version it doesn't have, or no older
    /// version to compare with, [`Error::UnknownVersion`]; a section in
    /// neither version [`Error::NoSuchSection`]. An index older than its
    /// maximum age is [`Error::StaleIndex`] unless `options.allow_stale`
    /// is set.
    pub fn diff(&self, document: &str, options: &DiffOptions<'_>) -> Result<DiffResult> {
        self.check_fresh(options.allow_stale)?;
        let mut name = document.trim().to_owned();
        let mut versions = self.sources(&name, true)?;
        if versions.is_empty()
            && let Some(doc) = self.aliases()?.get(&name)
        {
            name = doc.to_owned();
            versions = self.sources(&name, true)?;
        }
        if versions.is_empty() {
            return Err(Error::UnknownSource {
                name,
                known: self.source_names(true)?,
            });
        }
        let known: Vec<String> = versions.iter().map(pinned).collect();
        let unknown = |version: &str| Error::UnknownVersion {
            name: name.clone(),
            version: version.into(),
            known: known.clone(),
        };
        let aliases = self.aliases()?;
        let pick = |asked: &str| -> Result<&SourceRow> {
            let mut hex = asked.trim();
            // A pinned version, as errors and `older_versions` list it:
            // `doc@sha256:…`, the document by name or alias.
            if let Some((doc, rest)) = hex.rsplit_once('@') {
                if doc != name && aliases.get(doc) != Some(name.as_str()) {
                    return Err(unknown(asked));
                }
                hex = rest;
            }
            let hex = hex.strip_prefix("sha256:").unwrap_or(hex).to_lowercase();
            let mut found = versions.iter().filter(|s| s.revision.starts_with(&hex));
            match (hex.chars().all(|c| c.is_ascii_hexdigit()) && !hex.is_empty())
                .then(|| (found.next(), found.next()))
            {
                Some((Some(one), None)) => Ok(one),
                _ => Err(unknown(asked)),
            }
        };
        let current = self.current_version(&name)?.map(|s| s.id);
        let to = match options.to {
            Some(v) => pick(v)?,
            None => versions
                .iter()
                .find(|s| Some(s.id) == current)
                .ok_or_else(|| unknown("the current version"))?,
        };
        // With `to` asked for, the default `from` is pinned before it, so
        // the diff never runs backwards.
        let from = match options.from {
            Some(v) => pick(v)?,
            None => versions
                .iter()
                .rev()
                .filter(|s| options.to.is_none() || s.id < to.id)
                .find(|s| s.id != to.id && Some(s.id) != current)
                .ok_or_else(|| unknown("an older version"))?,
        };

        let old_sections = self.diff_sections(from.id)?;
        let new_sections = self.diff_sections(to.id)?;
        let mut old_used = vec![false; old_sections.len()];
        let mut pairs: Vec<(Option<&Section>, Option<&Section>)> = Vec::new();
        for n in &new_sections {
            let i = (0..old_sections.len())
                .find(|&i| !old_used[i] && old_sections[i].number == n.number)
                .or_else(|| {
                    (0..old_sections.len()).find(|&i| {
                        !old_used[i] && n.anchor.is_some() && old_sections[i].anchor == n.anchor
                    })
                });
            if let Some(i) = i {
                old_used[i] = true;
            }
            pairs.push((i.map(|i| &old_sections[i]), Some(n)));
        }
        for (i, o) in old_sections.iter().enumerate() {
            if !old_used[i] {
                pairs.push((Some(o), None));
            }
        }
        let asked = options.section.map(section_asked);
        if let Some(asked) = &asked {
            pairs.retain(|(o, n)| {
                o.is_some_and(|s| &s.number == asked) || n.is_some_and(|s| &s.number == asked)
            });
            if pairs.is_empty() {
                let candidates = match asked.parse::<SectionNumber>() {
                    Ok(number) => self.nearest_sections(to, &number)?,
                    Err(_) => Vec::new(),
                };
                return Err(Error::NoSuchSection {
                    document: pinned(to),
                    section: asked.clone(),
                    candidates,
                });
            }
        }

        let (from_pin, to_pin) = (pinned(from), pinned(to));
        let mut changes = Vec::new();
        let mut reworded = Vec::new();
        for (o, n) in pairs {
            let units = |s: Option<&Section>| {
                s.map(|s| version_units(&s.text, s.start_line, &s.steps))
                    .unwrap_or_default()
            };
            let (old_units, new_units) = (units(o), units(n));
            let section_cite =
                |pin: &str, s: Option<&Section>| s.map(|s| format!("{pin}#section-{}", s.number));
            let unit_cite = |pin: &str, s: &Section, u: &Unit| match &u.step {
                Some(k) => format!("{pin}#section-{}/step-{k}", s.number),
                None => format!("{pin}#section-{}", s.number),
            };
            let number = n.or(o).map(|s| s.number.clone()).unwrap_or_default();
            let (paired, removed, added) = pair(&old_units, &new_units);
            let mut by_new: Vec<(usize, VersionChange)> = Vec::new();
            for (i, j) in paired {
                let (ou, nu) = (&old_units[i], &new_units[j]);
                let (Some(os), Some(ns)) = (o, n) else {
                    continue;
                };
                if let Some(change) = classify(ou, nu) {
                    by_new.push((
                        j,
                        VersionChange {
                            change: change.into(),
                            section: number.clone(),
                            from: Some(unit_cite(&from_pin, os, ou)),
                            to: Some(unit_cite(&to_pin, ns, nu)),
                            from_text: Some(ou.text.clone()),
                            to_text: Some(nu.text.clone()),
                            from_strength: Some(strength(&ou.keywords)),
                            to_strength: Some(strength(&nu.keywords)),
                            text_truncated: false,
                        },
                    ));
                }
            }
            for j in added {
                let (nu, Some(ns)) = (&new_units[j], n) else {
                    continue;
                };
                if nu.is_normative() {
                    by_new.push((
                        j,
                        VersionChange {
                            change: "added".into(),
                            section: number.clone(),
                            from: section_cite(&from_pin, o),
                            to: Some(unit_cite(&to_pin, ns, nu)),
                            from_text: None,
                            to_text: Some(nu.text.clone()),
                            from_strength: None,
                            to_strength: Some(strength(&nu.keywords)),
                            text_truncated: false,
                        },
                    ));
                }
            }
            by_new.sort_by_key(|(j, _)| *j);
            let mut section_changes: Vec<VersionChange> =
                by_new.into_iter().map(|(_, c)| c).collect();
            for i in removed {
                let (ou, Some(os)) = (&old_units[i], o) else {
                    continue;
                };
                if ou.is_normative() {
                    section_changes.push(VersionChange {
                        change: "removed".into(),
                        section: number.clone(),
                        from: Some(unit_cite(&from_pin, os, ou)),
                        to: section_cite(&to_pin, n),
                        from_text: Some(ou.text.clone()),
                        to_text: None,
                        from_strength: Some(strength(&ou.keywords)),
                        to_strength: None,
                        text_truncated: false,
                    });
                }
            }
            for c in section_changes {
                if c.change == "reworded" {
                    reworded.push(c);
                } else {
                    changes.push(c);
                }
            }
        }
        let (total, reworded_total) = (changes.len(), reworded.len());
        let mut budget = MAX_DIFF_BYTES;
        let changes_cut = keep_within(&mut changes, &mut budget);
        let reworded_cut = keep_within(&mut reworded, &mut budget);
        let texts_cut = changes.iter().chain(&reworded).any(|c| c.text_truncated);
        let instruction = if texts_cut {
            QUOTE_TRUNCATED_INSTRUCTION
        } else {
            QUOTE_INSTRUCTION
        };
        Ok(DiffResult {
            document: name,
            from: from_pin,
            to: to_pin,
            section: asked,
            license_class: to.license_class.clone(),
            indexed_at: to.indexed_at.clone(),
            changes,
            reworded,
            total,
            reworded_total,
            truncated: changes_cut || reworded_cut,
            instruction: (to.license_class == "quote").then(|| instruction.into()),
        })
    }

    /// A document version's sections with their steps, in document order.
    /// A section whose number has no canonical citation form, such as an
    /// appendix's `A.1`, is left out, so every change's citation reads back.
    fn diff_sections(&self, source_id: i64) -> Result<Vec<Section>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.id, c.number, c.anchor, c.start_line, c.text
             FROM chunk c JOIN document d ON d.id = c.document_id
             WHERE d.source_id = ?1 AND c.kind = 'section' ORDER BY c.id",
        )?;
        let rows = stmt.query_map(params![source_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                Section {
                    number: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    anchor: row.get(2)?,
                    start_line: row.get(3)?,
                    text: row.get(4)?,
                    steps: Vec::new(),
                },
            ))
        })?;
        let mut sections: Vec<(i64, Section)> = rows.collect::<rusqlite::Result<_>>()?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.parent_id, c.number, c.start_line, c.end_line, c.text
             FROM chunk c JOIN document d ON d.id = c.document_id
             WHERE d.source_id = ?1 AND c.kind = 'step' ORDER BY c.id",
        )?;
        let rows = stmt.query_map(params![source_id], |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?,
                StepText {
                    number: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    start_line: row.get(2)?,
                    end_line: row.get(3)?,
                    text: row.get(4)?,
                },
            ))
        })?;
        let index: HashMap<i64, usize> = sections
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i))
            .collect();
        for row in rows {
            let (parent, step) = row?;
            if let Some(&i) = parent.and_then(|p| index.get(&p)) {
                sections[i].1.steps.push(step);
            }
        }
        Ok(sections
            .into_iter()
            .map(|(_, s)| s)
            .filter(|s| s.number.parse::<SectionNumber>().is_ok())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkRecord, DocumentRecord, SourceRecord};

    fn unit(text: &str) -> Unit {
        let units = version_units(&format!("1. Heading\n\n{text}"), 1, &[]);
        assert_eq!(units.len(), 1, "{units:?}");
        units.into_iter().next().unwrap()
    }

    #[test]
    fn a_definition_term_joins_its_description() {
        let units = version_units(
            "2.4. Logout Token\n\nThe following Claims are used:\n\niat\nREQUIRED. Issued at time.\nexp\nREQUIRED. Expiration time.",
            10,
            &[],
        );
        let texts: Vec<&str> = units.iter().map(|u| u.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "The following Claims are used:",
                "iat\nREQUIRED. Issued at time.",
                "exp\nREQUIRED. Expiration time.",
            ]
        );
        assert_eq!(units[2].term.as_deref(), Some("exp"));
        assert_eq!(units[2].keywords, [Keyword::Required]);
        assert_eq!(units[0].term, None);
    }

    #[test]
    fn a_step_is_one_unit_and_its_lines_belong_to_no_other() {
        let steps = [StepText {
            number: "1".into(),
            start_line: 13,
            end_line: 13,
            text: "1. Validate the iss Claim.".into(),
        }];
        let units = version_units(
            "2.6. Validation\n\nThe RP MUST validate it:\n1. Validate the iss Claim.\n\nOtherwise, reject it.",
            10,
            &steps,
        );
        let texts: Vec<(&str, Option<&str>)> = units
            .iter()
            .map(|u| (u.text.as_str(), u.step.as_deref()))
            .collect();
        assert_eq!(
            texts,
            [
                ("The RP MUST validate it:", None),
                ("1. Validate the iss Claim.", Some("1")),
                ("Otherwise, reject it.", None),
            ]
        );
        assert!(units[1].is_normative());
        assert!(!units[2].is_normative());
    }

    #[test]
    fn indented_lines_up_to_a_blank_line_are_one_unit() {
        let units = version_units(
            "2.3.1. Client Password\n\n   client_id\n         REQUIRED.  The client\n         identifier.\n\n   client_secret\n         OPTIONAL.",
            1,
            &[],
        );
        assert_eq!(units.len(), 2, "{units:?}");
        assert_eq!(
            units[0].text,
            "   client_id\n         REQUIRED.  The client\n         identifier."
        );
        assert_eq!(units[0].term.as_deref(), Some("client_id"));
    }

    #[test]
    fn content_keys_drop_punctuation_case_months_and_years() {
        assert_eq!(
            unit("REQUIRED. As in [OpenID.Core] (Core 1.0, November 2014.).").key,
            ["required", "as", "in", "openid", "core", "core", "1", "0"]
        );
    }

    #[test]
    fn a_reference_date_is_rewording() {
        let old = unit("iat\nREQUIRED. Issued at time, per [Core] (Core, November 2014.).");
        let new = unit("iat\nREQUIRED. Issued at time, per [Core] (Core, December 2023.).");
        assert_eq!(classify(&old, &new), Some("reworded"));
    }

    #[test]
    fn a_reference_annotation_or_renumbered_cross_reference_is_rewording() {
        let old = unit(
            "Clients MUST use TLS as in [RFC6749] (Hardt, D., Ed., “The OAuth 2.0 Framework,” October 2012.). See Section 7.1 (TLS Requirements).",
        );
        let new = unit("Clients MUST use TLS as in [RFC6749]. See Section 6.1 (TLS Requirements).");
        assert_eq!(classify(&old, &new), Some("reworded"));
        // A title may hold parentheses of its own.
        let jws = unit(
            "Clients MUST use TLS as in [RFC6749] (Jones, M., “JSON Web Signature (JWS),” May 2015.). See Section 6.1 (TLS Requirements).",
        );
        assert_eq!(classify(&jws, &new), Some("reworded"));
        // Another reference, or another section, is a change.
        let other =
            unit("Clients MUST use TLS as in [RFC9110]. See Section 6.1 (TLS Requirements).");
        assert_eq!(classify(&new, &other), Some("changed"));
        let other = unit("Clients MUST use TLS as in [RFC6749]. See Section 6.1 (Security).");
        assert_eq!(classify(&new, &other), Some("changed"));
    }

    #[test]
    fn an_opposite_is_never_a_spelling_fix() {
        for (old, new) in [
            (
                "The Logout Token MUST be signed.",
                "The Logout Token MUST be unsigned.",
            ),
            (
                "The response MUST include it.",
                "The response MUST exclude it.",
            ),
            ("The client MUST encrypt it.", "The client MUST decrypt it."),
            (
                "The token MUST be valid here.",
                "The token MUST be invalid here.",
            ),
        ] {
            assert_eq!(classify(&unit(old), &unit(new)), Some("changed"), "{new}");
        }
    }

    #[test]
    fn reordered_words_are_a_change() {
        let old = unit("X MUST be signed and Y MAY be encrypted.");
        let new = unit("X MAY be signed and Y MUST be encrypted.");
        assert_eq!(classify(&old, &new), Some("changed"));
        let old = unit("The client MUST NOT send it to the server.");
        let new = unit("The server MUST NOT send it to the client.");
        assert_eq!(classify(&old, &new), Some("changed"));
    }

    #[test]
    fn a_number_is_a_year_only_after_a_month() {
        let old = unit("The OP MUST NOT wait more than 2000 seconds.");
        let new = unit("The OP MUST NOT wait more than 2048 seconds.");
        assert_eq!(classify(&old, &new), Some("changed"));
        // A quoted value in a parenthesis is no reference tooltip.
        let old = unit("It MUST NOT use an alg (the value “none”).");
        let new = unit("It MUST NOT use an alg (the value “HS256”).");
        assert_eq!(classify(&old, &new), Some("changed"));
    }

    #[test]
    fn a_spelling_fix_is_rewording() {
        let old = unit("The response MAY say what error occured.");
        let new = unit("The response MAY say what error occurred.");
        assert_eq!(classify(&old, &new), Some("reworded"));
    }

    #[test]
    fn a_keyword_change_is_a_strength_change() {
        let old = unit("sub\nOPTIONAL. Subject Identifier.");
        let new = unit("sub\nREQUIRED. Subject Identifier.");
        assert_eq!(classify(&old, &new), Some("strength_changed"));
    }

    #[test]
    fn a_new_word_is_a_change() {
        let steps = |text: &str| {
            let step = StepText {
                number: "4".into(),
                start_line: 3,
                end_line: 3,
                text: text.into(),
            };
            version_units(&format!("2.6. V\n\n{text}"), 1, &[step]).remove(0)
        };
        let old = steps("4. Validate the iss, aud, and iat Claims.");
        let new = steps("4. Validate the iss, aud, iat, and exp Claims.");
        assert_eq!(classify(&old, &new), Some("changed"));
        // A short identifier never passes for a spelling fix.
        let old = unit("A token MUST contain a sub Claim.");
        let new = unit("A token MUST contain a sid Claim.");
        assert_eq!(classify(&old, &new), Some("changed"));
    }

    #[test]
    fn identical_and_prose_pairs_are_not_reported() {
        let same = unit("A Logout Token MUST be signed.");
        assert_eq!(classify(&same, &same.clone()), None);
        let old = unit("This is an example.");
        let new = unit("This is another example.");
        assert_eq!(classify(&old, &new), None);
    }

    #[test]
    fn different_terms_never_pair() {
        let old = [unit("iat\nREQUIRED. Issued at time, as in Section 2.")];
        let new = [
            unit("iat\nREQUIRED. Issued at time, as in Section 2."),
            unit("exp\nREQUIRED. Expiration time, as in Section 2."),
        ];
        assert_eq!(pair(&old, &new), (vec![(0, 0)], vec![], vec![1]));
        let old = [unit("iat\nREQUIRED. Issued at time, as in Section 2.")];
        let new = [unit("exp\nREQUIRED. Expiration time, as in Section 2.")];
        assert_eq!(pair(&old, &new), (vec![], vec![0], vec![0]));
    }

    #[test]
    fn similar_units_pair_and_dissimilar_ones_do_not() {
        let old = [
            unit("Logout Tokens MAY contain other Claims."),
            unit("A Logout Token MUST be signed and MAY be encrypted."),
        ];
        let new = [
            unit("The RP MUST respond with HTTP 200."),
            unit("A Logout Token MUST be signed and MAY also be encrypted."),
        ];
        assert_eq!(pair(&old, &new), (vec![(1, 1)], vec![0], vec![0]));
    }

    // Store-level tests, over versions written through `Store`.

    const NEW: &str = "3b5c933d5269047b39a65166d5c4d8ff6852b06451f54ed2531b8f97299fdc50";
    const OLD: &str = "02343424f9e52edf8c78173540e25da913ad2784ea292dbf7980222ab1b18a22";
    const DOC: &str = "openid-connect-backchannel-1_0";

    fn source<'a>(name: &'a str, revision: &'a str, current: bool) -> SourceRecord<'a> {
        SourceRecord {
            name,
            kind: "document",
            url: "https://example.com",
            revision,
            license_spdx: None,
            license_class: "quote",
            is_current: current,
            obsoleted_by: None,
            updated_by: None,
        }
    }

    /// A section chunk starting at line `start`.
    fn section<'a>(number: &'a str, start: u32, text: &'a str) -> ChunkRecord<'a> {
        ChunkRecord {
            kind: "section",
            start_line: start,
            end_line: start + u32::try_from(text.lines().count()).unwrap() - 1,
            text,
            number: Some(number),
            anchor: Some(number),
            title: Some(number),
            parent: None,
            strength: &[],
            symbol: None,
        }
    }

    fn write(store: &mut Store, revision: &str, current: bool, chunks: Vec<ChunkRecord<'_>>) {
        let id = store
            .upsert_source(&source(DOC, revision, current))
            .unwrap();
        let doc = DocumentRecord {
            path: DOC,
            language: None,
            content_hash: revision.as_bytes(),
            size_bytes: 1,
            dirty: false,
            text: None,
            chunks,
        };
        store.write_document(id, &doc).unwrap();
    }

    struct Fixture {
        store: Store,
        _dir: tempfile::TempDir,
    }

    const OLD_2_4: &str = "2.4. Logout Token\n\nsub\nOPTIONAL. Subject, November 2014.\niat\nREQUIRED. Issued at, November 2014.\n\nLogout Tokens MAY contain other Claims.";
    const NEW_2_4: &str = "2.4. Logout Token\n\nsub\nREQUIRED. Subject, December 2023.\niat\nREQUIRED. Issued at, December 2023.\nexp\nREQUIRED. Expiration time, December 2023.";

    /// Back-Channel Logout at two versions: §2.4 changed, §2.5 the same,
    /// §2.9 only in the new one and §2.10 only in the old.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        write(
            &mut store,
            OLD,
            false,
            vec![
                section("2.4", 1, OLD_2_4),
                section("2.5", 20, "2.5. Request\n\nThe OP MUST POST it."),
                section("2.10", 30, "2.10. Gone\n\nThe RP MUST forget it."),
            ],
        );
        write(
            &mut store,
            NEW,
            true,
            vec![
                section("2.4", 1, NEW_2_4),
                section("2.5", 20, "2.5. Request\n\nThe OP MUST POST it."),
                section("2.9", 30, "2.9. New\n\nThe RP SHOULD log it."),
            ],
        );
        let mut aliases = crate::SpecAliases::new();
        aliases.insert("Back-Channel Logout", DOC);
        store.set_aliases(&aliases).unwrap();
        store.set_indexed_at(30).unwrap();
        Fixture { store, _dir: dir }
    }

    fn diff(f: &Fixture, section: Option<&str>) -> DiffResult {
        f.store
            .diff(
                DOC,
                &DiffOptions {
                    section,
                    ..DiffOptions::default()
                },
            )
            .unwrap()
    }

    const NEW_PIN: &str = "openid-connect-backchannel-1_0@sha256:3b5c933d5269";
    const OLD_PIN: &str = "openid-connect-backchannel-1_0@sha256:02343424f9e5";

    #[test]
    fn a_section_diff_reports_added_removed_and_strength_changes() {
        let d = diff(&fixture(), Some("2.4"));
        assert_eq!((d.from.as_str(), d.to.as_str()), (OLD_PIN, NEW_PIN));
        assert_eq!(d.section.as_deref(), Some("2.4"));
        let summary: Vec<(&str, Option<&str>, Option<&str>)> = d
            .changes
            .iter()
            .map(|c| {
                (
                    c.change.as_str(),
                    c.from_text.as_deref(),
                    c.to_text.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "strength_changed",
                    Some("sub\nOPTIONAL. Subject, November 2014."),
                    Some("sub\nREQUIRED. Subject, December 2023.")
                ),
                (
                    "added",
                    None,
                    Some("exp\nREQUIRED. Expiration time, December 2023.")
                ),
                (
                    "removed",
                    Some("Logout Tokens MAY contain other Claims."),
                    None
                ),
            ]
        );
        let added = &d.changes[1];
        assert_eq!(
            added.from.as_deref(),
            Some(&*format!("{OLD_PIN}#section-2.4"))
        );
        assert_eq!(
            added.to.as_deref(),
            Some(&*format!("{NEW_PIN}#section-2.4"))
        );
        assert_eq!(
            added.to_strength.as_deref(),
            Some(&["REQUIRED".to_owned()][..])
        );
        assert_eq!(added.from_strength, None);
        assert_eq!(
            d.changes[0].from_strength.as_deref(),
            Some(&["OPTIONAL".to_owned()][..])
        );
        // iat changed only its reference date.
        assert_eq!(d.reworded.len(), 1);
        assert_eq!(d.reworded[0].change, "reworded");
        assert_eq!(d.total, 3);
        assert!(!d.truncated);
        assert_eq!(d.instruction.as_deref(), Some(QUOTE_INSTRUCTION));
    }

    #[test]
    fn an_unchanged_section_reports_nothing() {
        let d = diff(&fixture(), Some("2.5"));
        assert!(d.changes.is_empty(), "{:?}", d.changes);
        assert!(d.reworded.is_empty());
        assert_eq!(d.total, 0);
    }

    #[test]
    fn a_section_only_one_version_has_is_all_added_or_removed() {
        let d = diff(&fixture(), Some("2.9"));
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].change, "added");
        assert_eq!(d.changes[0].from, None);
        assert_eq!(
            d.changes[0].to.as_deref(),
            Some(&*format!("{NEW_PIN}#section-2.9"))
        );
        let d = diff(&fixture(), Some("§2.10"));
        assert_eq!(d.changes[0].change, "removed");
        assert_eq!(d.changes[0].section, "2.10");
        assert_eq!(d.changes[0].to, None);
    }

    #[test]
    fn a_whole_document_diff_covers_every_section() {
        let d = diff(&fixture(), None);
        let sections: Vec<&str> = d.changes.iter().map(|c| c.section.as_str()).collect();
        assert_eq!(sections, ["2.4", "2.4", "2.4", "2.9", "2.10"]);
        assert_eq!(d.section, None);
    }

    #[test]
    fn versions_can_be_named_and_an_alias_names_the_document() {
        let f = fixture();
        let options = DiffOptions {
            from: Some("sha256:3b5c933d5269"),
            to: Some("3b5c933d"),
            ..DiffOptions::default()
        };
        let d = f.store.diff("Back-Channel Logout", &options).unwrap();
        assert_eq!((d.from.as_str(), d.to.as_str()), (NEW_PIN, NEW_PIN));
        assert!(d.changes.is_empty());
        let swapped = DiffOptions {
            from: Some(NEW),
            to: Some(OLD),
            section: Some("2.4"),
            ..DiffOptions::default()
        };
        let d = f.store.diff(DOC, &swapped).unwrap();
        let changes: Vec<&str> = d.changes.iter().map(|c| c.change.as_str()).collect();
        assert_eq!(changes, ["strength_changed", "added", "removed"]);
        assert!(
            d.changes[2]
                .from_text
                .as_deref()
                .unwrap()
                .starts_with("exp\n")
        );
    }

    #[test]
    fn unknown_documents_versions_and_sections_are_errors() {
        let f = fixture();
        let err = f
            .store
            .diff("rfc9999", &DiffOptions::default())
            .unwrap_err();
        assert!(matches!(err, Error::UnknownSource { .. }), "{err}");
        let err = f
            .store
            .diff(
                DOC,
                &DiffOptions {
                    from: Some("sha256:ffffffffffff"),
                    ..DiffOptions::default()
                },
            )
            .unwrap_err();
        let Error::UnknownVersion { known, .. } = err else {
            panic!("{err}");
        };
        assert_eq!(known, [OLD_PIN, NEW_PIN]);
        let err = f
            .store
            .diff(
                DOC,
                &DiffOptions {
                    section: Some("7.1"),
                    ..DiffOptions::default()
                },
            )
            .unwrap_err();
        assert!(matches!(err, Error::NoSuchSection { .. }), "{err}");
    }

    #[test]
    fn a_pinned_version_as_errors_and_results_list_it_names_a_version() {
        let f = fixture();
        let options = DiffOptions {
            from: Some(OLD_PIN),
            to: Some(NEW_PIN),
            section: Some("2.4"),
            ..DiffOptions::default()
        };
        let d = f.store.diff("Back-Channel Logout", &options).unwrap();
        assert_eq!((d.from.as_str(), d.to.as_str()), (OLD_PIN, NEW_PIN));
        // Another document's pin is not one of this one's versions.
        let options = DiffOptions {
            from: Some("rfc9449@sha256:02343424f9e5"),
            ..DiffOptions::default()
        };
        assert!(matches!(
            f.store.diff(DOC, &options),
            Err(Error::UnknownVersion { .. })
        ));
    }

    #[test]
    fn the_default_from_is_older_than_an_asked_for_to() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("three.db")).unwrap();
        let v1 = "a1".repeat(32);
        let v2 = "b2".repeat(32);
        for (rev, current) in [(v1.as_str(), false), (v2.as_str(), false), (NEW, true)] {
            write(
                &mut store,
                rev,
                current,
                vec![section("2.5", 1, "2.5. R\n\nX MUST.")],
            );
        }
        store.set_indexed_at(30).unwrap();
        let to_v2 = DiffOptions {
            to: Some(&v2),
            ..DiffOptions::default()
        };
        let d = store.diff(DOC, &to_v2).unwrap();
        assert_eq!(d.from, format!("{DOC}@sha256:{}", &v1[..12]));
        let to_v1 = DiffOptions {
            to: Some(&v1),
            ..DiffOptions::default()
        };
        let err = store.diff(DOC, &to_v1).unwrap_err();
        assert!(matches!(err, Error::UnknownVersion { .. }), "{err}");
    }

    #[test]
    fn long_texts_and_long_diffs_say_they_are_cut() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("big.db")).unwrap();
        let long = format!("2.5. R\n\nThe client MUST {}.", "send it ".repeat(1000));
        write(
            &mut store,
            OLD,
            false,
            vec![section("2.5", 1, "2.5. R\n\nNothing.")],
        );
        let many: String = (0..200)
            .map(|i| format!("Rule {i}: the client MUST do thing number {i} quickly{i}.\n"))
            .collect();
        let many = format!("2.6. S\n\n{many}");
        write(
            &mut store,
            NEW,
            true,
            vec![section("2.5", 1, &long), section("2.6", 10, &many)],
        );
        store.set_indexed_at(30).unwrap();
        let d = store.diff(DOC, &DiffOptions::default()).unwrap();
        let first = &d.changes[0];
        assert!(first.text_truncated, "{first:?}");
        assert!(first.to_text.as_deref().unwrap().len() <= MAX_SNIPPET_BYTES);
        assert_eq!(
            d.instruction.as_deref(),
            Some(crate::QUOTE_TRUNCATED_INSTRUCTION)
        );
        assert_eq!(d.total, 201);
        assert!(d.truncated);
        let bytes: usize = d
            .changes
            .iter()
            .chain(&d.reworded)
            .map(|c| {
                c.from_text.as_deref().map_or(0, str::len)
                    + c.to_text.as_deref().map_or(0, str::len)
            })
            .sum();
        assert!(bytes <= crate::MAX_READ_BYTES, "{bytes}");
    }

    #[test]
    fn a_huge_section_still_diffs_in_bounded_work() {
        let old: Vec<Unit> = (0..3000)
            .map(|i| unit(&format!("The client MUST send value {i}.")))
            .collect();
        let new: Vec<Unit> = (0..3000)
            .map(|i| unit(&format!("The client MUST send value {}.", i + 5000)))
            .collect();
        let started = std::time::Instant::now();
        let (pairs, old_left, new_left) = pair(&old, &new);
        assert_eq!(pairs.len() + old_left.len(), 3000);
        assert_eq!(pairs.len() + new_left.len(), 3000);
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    }

    #[test]
    fn a_document_with_one_version_has_nothing_to_diff_against() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("one.db")).unwrap();
        write(
            &mut store,
            NEW,
            true,
            vec![section("2.5", 1, "2.5. R\n\nX MUST.")],
        );
        store.set_indexed_at(30).unwrap();
        let err = store.diff(DOC, &DiffOptions::default()).unwrap_err();
        let Error::UnknownVersion { version, known, .. } = err else {
            panic!("{err}");
        };
        assert_eq!(version, "an older version");
        assert_eq!(known, [NEW_PIN]);
    }
}
