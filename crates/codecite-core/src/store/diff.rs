//! Diffing two pinned versions of a spec document: the normative
//! statements one has that the other lacks, or has at another strength.
//!
//! See `docs/superpowers/specs/2026-10-28-spec-version-diff-design.md`. The
//! statement extraction is [`version_units`], kept in one function so that
//! it can be unified with other normative-statement extraction later.

use std::collections::{HashMap, VecDeque};
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

/// A step's unit. Its key leaves out the number the step starts with, so a
/// step renumbered by an insertion above it is no change.
fn step_unit(step: &StepText) -> Unit {
    let mut unit = new_unit(step.text.clone(), Some(step.number.clone()), None);
    if unit.key.first() == Some(&step.number) {
        unit.key.remove(0);
    }
    unit
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
/// dropped and its title kept, so a renumbering is no change. A tooltip is
/// only a parenthesis with a quoted title that ends in a month and a year;
/// any other date or number, in a rule or in a parenthesis, is content.
fn content_key(text: &str) -> Vec<String> {
    static ANNOTATION: LazyLock<Regex> = LazyLock::new(|| {
        let months = MONTHS.join("|");
        Regex::new(&format!(
            r"(?i:\([^()“]*“[^”]*”[^()]*\b(?:{months})\s+(?:19|20)\d{{2}}\.?\))|Section\s+[0-9A-Z][0-9.]*\s*(?:\()"
        ))
        .expect("a valid regex")
    });
    let text = ANNOTATION.replace_all(text, |c: &regex::Captures<'_>| {
        // A cross-reference keeps its parenthesis, whose title follows.
        if c[0].starts_with('(') { " " } else { " (" }
    });
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
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

/// Whether two keys say the same words in the same order but for spelling
/// fixes: each word that differs, at its place, is one edit from the other,
/// both are letters only and at least five long, and neither is the other
/// with a letter added at an end. Short words are identifiers (`sub`,
/// `sid`); words with digits are values (`RS256`, `PS256`; `86400`); two
/// edits already reach opposites (`signed`, `unsigned`); and words in
/// another order are a change, since moving a word moves a rule.
fn spelling_only(old: &[String], new: &[String]) -> bool {
    old.len() == new.len()
        && old != new
        && old
            .iter()
            .zip(new)
            .all(|(a, b)| a == b || is_spelling_fix(a, b))
}

/// Whether `b` is `a` with its spelling fixed: a letter doubled or
/// undoubled (`occured`, `occurred`) or two neighbouring letters swapped
/// (`recieve`, `receive`), in words of letters only at least five long.
/// Any other one-letter edit makes another word as often as it fixes one
/// (`issuer`, `issued`; `inline`, `online`; `server`, `sever`).
fn is_spelling_fix(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().min(b.len()) < 5 || !a.iter().chain(&b).all(|c| c.is_alphabetic()) || a == b {
        return false;
    }
    let (short, long) = if a.len() < b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    match long.len() - short.len() {
        // Doubled: dropping one of two equal neighbours gives the other.
        1 => (1..long.len()).any(|k| {
            long[k] == long[k - 1] && long[..k] == short[..k] && long[k + 1..] == short[k..]
        }),
        // Swapped: they differ at exactly two neighbouring places, crosswise.
        0 => {
            let differ: Vec<usize> = (0..a.len()).filter(|&k| a[k] != b[k]).collect();
            matches!(differ[..], [k, l] if l == k + 1 && a[k] == b[l] && a[l] == b[k])
        }
        _ => false,
    }
}

fn counts(words: &[String]) -> HashMap<&str, usize> {
    let mut c = HashMap::new();
    for w in words {
        *c.entry(w.as_str()).or_insert(0) += 1;
    }
    c
}

/// The Dice coefficient of two keys' word multisets, from their word
/// counts and lengths: from 0, nothing in common, to 1, the same words.
fn dice(a: &HashMap<&str, usize>, a_len: usize, b: &HashMap<&str, usize>, b_len: usize) -> f64 {
    if a_len + b_len == 0 {
        return 1.0;
    }
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let common: usize = small
        .iter()
        .map(|(w, n)| (*n).min(large.get(w).copied().unwrap_or(0)))
        .sum();
    #[expect(clippy::cast_precision_loss, reason = "word counts are small")]
    let score = 2.0 * common as f64 / (a_len + b_len) as f64;
    score
}

/// How many word lookups one whole diff spends scoring units by
/// similarity. A section that would take more, or a diff past it, reports
/// what is left unpaired as added and removed.
const MAX_SIMILARITY_WORK: usize = 20_000_000;

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

/// A unit's key, whether it is a step, and its term: units pair by exact
/// key only within one bucket.
type Bucket<'a> = (&'a [String], bool, Option<&'a str>);

fn bucket(u: &Unit) -> Bucket<'_> {
    (u.key.as_slice(), u.step.is_some(), u.term.as_deref())
}

/// Pairs the units of two versions: the same content first, in order, then
/// the most similar. Returns the pairs, by new index, then the unpaired old
/// and new indexes.
#[cfg(test)]
fn pair(old: &[Unit], new: &[Unit]) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    pair_within(old, new, &mut MAX_SIMILARITY_WORK.clone())
}

/// [`pair`], spending at most `work` word lookups on similarity, and
/// taking what it spends from it.
fn pair_within(
    old: &[Unit],
    new: &[Unit],
    work: &mut usize,
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    let mut old_used = vec![false; old.len()];
    let mut new_used = vec![false; new.len()];
    let mut pairs = Vec::new();
    // Units that may pair with the same key share a queue, so each new one
    // takes the first unused old one in constant time.
    let mut by_key: HashMap<Bucket<'_>, VecDeque<usize>> = HashMap::new();
    for (i, o) in old.iter().enumerate() {
        by_key.entry(bucket(o)).or_default().push_back(i);
    }
    for (j, n) in new.iter().enumerate() {
        if let Some(i) = by_key.get_mut(&bucket(n)).and_then(VecDeque::pop_front) {
            old_used[i] = true;
            new_used[j] = true;
            pairs.push((i, j));
        }
    }
    let mut candidates = Vec::new();
    let left = |used: &[bool]| used.iter().filter(|u| !**u).count();
    // Scoring every pair is quadratic; past a bound, what is left unpaired
    // is reported as added and removed rather than paired by similarity.
    let mut bounded = left(&old_used).saturating_mul(left(&new_used)) <= MAX_SIMILARITY_PAIRS;
    let open_old: Vec<usize> = (0..old.len()).filter(|&i| !old_used[i]).collect();
    let open_new: Vec<usize> = (0..new.len()).filter(|&j| !new_used[j]).collect();
    if bounded {
        // A pair costs a lookup per distinct word of its shorter key.
        let cost: usize = open_old
            .iter()
            .flat_map(|&i| open_new.iter().map(move |&j| (i, j)))
            .filter(|&(i, j)| may_pair(&old[i], &new[j]))
            .map(|(i, j)| old[i].key.len().min(new[j].key.len()))
            .sum();
        bounded = cost <= *work;
        if bounded {
            *work -= cost;
        }
    }
    if bounded {
        let old_counts: Vec<_> = open_old.iter().map(|&i| counts(&old[i].key)).collect();
        let new_counts: Vec<_> = open_new.iter().map(|&j| counts(&new[j].key)).collect();
        for (oi, &i) in open_old.iter().enumerate() {
            for (nj, &j) in open_new.iter().enumerate() {
                if !may_pair(&old[i], &new[j]) {
                    continue;
                }
                let score = dice(
                    &old_counts[oi],
                    old[i].key.len(),
                    &new_counts[nj],
                    new[j].key.len(),
                );
                if score >= MIN_SIMILARITY {
                    candidates.push((score, i, j));
                }
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
        // With `to` an older version, the default `from` is pinned before
        // it, so the diff never runs backwards. The current version is the
        // newest wherever the manifest lists it.
        let from = match options.from {
            Some(v) => pick(v)?,
            None => versions
                .iter()
                .rev()
                .filter(|s| Some(to.id) == current || s.id < to.id)
                .find(|s| s.id != to.id && Some(s.id) != current)
                .ok_or_else(|| unknown("an older version"))?,
        };

        let old_sections = self.diff_sections(from.id)?;
        let new_sections = self.diff_sections(to.id)?;
        let mut old_used = vec![false; old_sections.len()];
        let mut pairs: Vec<(Option<&Section>, Option<&Section>)> = Vec::new();
        // An author's anchor (`LogoutToken`) follows a section when a
        // section inserted before it renumbers it; an anchor made from the
        // number (`section-2.5`, `rfc.section.2.5`) does not. So sections
        // pair by authored anchor first, then by number.
        let authored = |s: &Section| {
            s.anchor
                .clone()
                .filter(|a| !a.contains(s.number.as_str()) && !a.is_empty())
        };
        let mut matched: Vec<Option<usize>> = vec![None; new_sections.len()];
        for (k, n) in new_sections.iter().enumerate() {
            let Some(anchor) = authored(n) else { continue };
            if let Some(i) = (0..old_sections.len())
                .find(|&i| !old_used[i] && authored(&old_sections[i]).as_ref() == Some(&anchor))
            {
                old_used[i] = true;
                matched[k] = Some(i);
            }
        }
        for (k, n) in new_sections.iter().enumerate() {
            if matched[k].is_some() {
                continue;
            }
            if let Some(i) = (0..old_sections.len())
                .find(|&i| !old_used[i] && old_sections[i].number == n.number)
            {
                old_used[i] = true;
                matched[k] = Some(i);
            }
        }
        for (k, n) in new_sections.iter().enumerate() {
            pairs.push((matched[k].map(|i| &old_sections[i]), Some(n)));
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
        let mut work = MAX_SIMILARITY_WORK;
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
            let (paired, removed, added) = pair_within(&old_units, &new_units, &mut work);
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
    fn content_keys_drop_punctuation_case_and_reference_tooltips() {
        assert_eq!(
            unit("REQUIRED. As in [OpenID.Core] (Sakimura, N., “Core 1.0,” November 2014.).").key,
            ["required", "as", "in", "openid", "core"]
        );
        // A date outside a tooltip is content.
        assert_eq!(
            unit("REQUIRED. As in Core 1.0, November 2014.").key,
            ["required", "as", "in", "core", "1", "0", "november", "2014"]
        );
    }

    #[test]
    fn a_reference_date_is_rewording() {
        let old = unit(
            "iat\nREQUIRED. Issued at time, per [Core] (Sakimura, “Core 1.0,” November 2014.).",
        );
        let new = unit(
            "iat\nREQUIRED. Issued at time, per [Core] (Sakimura, “Core 1.0,” December 2023.).",
        );
        assert_eq!(classify(&old, &new), Some("reworded"));
    }

    #[test]
    fn a_date_or_number_in_a_rule_is_a_change() {
        let old = unit("Clients MUST NOT use it after January 2025.");
        let new = unit("Clients MUST NOT use it after January 2026.");
        assert_eq!(classify(&old, &new), Some("changed"));
        let old = unit("It MUST hold (the “expires_in” value MUST NOT exceed 3600.)");
        let new = unit("It MUST hold (the “expires_in” value MUST NOT exceed 7200.)");
        assert_eq!(classify(&old, &new), Some("changed"));
    }

    #[test]
    fn only_a_doubled_letter_or_a_transposition_is_a_spelling_fix() {
        for (old, new, change) in [
            (
                "The error that occured MUST be named.",
                "The error that occurred MUST be named.",
                "reworded",
            ),
            (
                "The RP MUST recieve it.",
                "The RP MUST receive it.",
                "reworded",
            ),
            (
                "The issuer MUST be named.",
                "The issued MUST be named.",
                "changed",
            ),
            (
                "It MUST be passed inline.",
                "It MUST be passed online.",
                "changed",
            ),
            (
                "The client MUST enable it.",
                "The client MUST unable it.",
                "changed",
            ),
            (
                "The server MUST sign it.",
                "The sever MUST sign it.",
                "changed",
            ),
        ] {
            assert_eq!(
                classify(&unit(old), &unit(new)),
                Some(change),
                "{old} → {new}"
            );
        }
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
    fn an_algorithm_or_number_one_character_apart_is_a_change() {
        let old = unit("The ID Token MUST be signed with RS256.");
        let new = unit("The ID Token MUST be signed with PS256.");
        assert_eq!(classify(&old, &new), Some("changed"));
        let old = unit("A token MUST NOT live past 86400 seconds.");
        let new = unit("A token MUST NOT live past 86401 seconds.");
        assert_eq!(classify(&old, &new), Some("changed"));
    }

    #[test]
    fn a_reordering_with_a_spelling_fix_is_a_change() {
        let old = unit("The client MUST NOT send it to the server, as occured.");
        let new = unit("The server MUST NOT send it to the client, as occurred.");
        assert_eq!(classify(&old, &new), Some("changed"));
    }

    #[test]
    fn a_renumbered_step_is_no_change() {
        let step = |number: &str, text: &str| {
            let s = StepText {
                number: number.into(),
                start_line: 3,
                end_line: 3,
                text: text.into(),
            };
            version_units(&format!("2.6. V\n\n{text}"), 1, &[s]).remove(0)
        };
        let old = step("4", "4. Validate the iss Claim.");
        let new = step("5", "5. Validate the iss Claim.");
        assert_eq!(old.key, new.key);
        assert_eq!(classify(&old, &new), Some("reworded"));
    }

    #[test]
    fn long_or_identical_units_pair_in_bounded_work() {
        let same: Vec<Unit> = (0..20_000).map(|_| unit("x MUST hold.")).collect();
        let started = std::time::Instant::now();
        let (pairs, ..) = pair(&same, &same);
        assert_eq!(pairs.len(), 20_000);
        let long = "a".repeat(200_000);
        let old = unit(&format!("It MUST be {long}b."));
        let new = unit(&format!("It MUST be {long}c."));
        assert_eq!(classify(&old, &new), Some("changed"));
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
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

    const OLD_2_4: &str = "2.4. Logout Token\n\nsub\nOPTIONAL. Subject, [Core] (S., “Core,” November 2014.).\niat\nREQUIRED. Issued at, [Core] (S., “Core,” November 2014.).\n\nLogout Tokens MAY contain other Claims.";
    const NEW_2_4: &str = "2.4. Logout Token\n\nsub\nREQUIRED. Subject, [Core] (S., “Core,” December 2023.).\niat\nREQUIRED. Issued at, [Core] (S., “Core,” December 2023.).\nexp\nREQUIRED. Expiration time, [Core] (S., “Core,” December 2023.).";

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
                    Some("sub\nOPTIONAL. Subject, [Core] (S., “Core,” November 2014.)."),
                    Some("sub\nREQUIRED. Subject, [Core] (S., “Core,” December 2023.).")
                ),
                (
                    "added",
                    None,
                    Some("exp\nREQUIRED. Expiration time, [Core] (S., “Core,” December 2023.).")
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
    fn naming_the_current_version_as_to_works_whatever_the_pinning_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("order.db")).unwrap();
        // The current version is pinned first.
        write(
            &mut store,
            NEW,
            true,
            vec![section("2.5", 1, "2.5. R\n\nX MUST.")],
        );
        write(
            &mut store,
            OLD,
            false,
            vec![section("2.5", 1, "2.5. R\n\nX SHOULD.")],
        );
        store.set_indexed_at(30).unwrap();
        let options = DiffOptions {
            to: Some(NEW_PIN),
            ..DiffOptions::default()
        };
        let d = store.diff(DOC, &options).unwrap();
        assert_eq!(d.from, OLD_PIN);
        assert_eq!(d.changes[0].change, "strength_changed");
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
    fn sections_pair_by_their_authors_anchor_before_their_number() {
        let chunk = |number: &'static str, anchor: &'static str, start: u32, text: &'static str| {
            ChunkRecord {
                anchor: Some(anchor),
                ..section(number, start, text)
            }
        };
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("moved.db")).unwrap();
        write(
            &mut store,
            OLD,
            false,
            vec![
                chunk("2.4", "LogoutToken", 1, "2.4. Token\n\nIt MUST be signed."),
                chunk(
                    "2.5",
                    "BCRequest",
                    10,
                    "2.5. Request\n\nThe OP MUST POST it.",
                ),
            ],
        );
        write(
            &mut store,
            NEW,
            true,
            vec![
                chunk("2.4", "LogoutToken", 1, "2.4. Token\n\nIt MUST be signed."),
                chunk("2.5", "Inserted", 10, "2.5. New\n\nThe RP SHOULD log it."),
                chunk(
                    "2.6",
                    "BCRequest",
                    20,
                    "2.6. Request\n\nThe OP MUST POST it.",
                ),
                chunk("2.7", "rfc.section.2.7", 30, "2.7. Plain\n\nX MAY."),
            ],
        );
        store.set_indexed_at(30).unwrap();
        let d = store.diff(DOC, &DiffOptions::default()).unwrap();
        let changes: Vec<(&str, &str)> = d
            .changes
            .iter()
            .map(|c| (c.change.as_str(), c.section.as_str()))
            .collect();
        assert_eq!(
            changes,
            [("added", "2.5"), ("added", "2.7")],
            "{:#?}",
            d.changes
        );
    }

    #[test]
    fn long_similar_units_pair_in_bounded_work() {
        let words: String = (0..2000).map(|i| format!("w{i} ")).collect();
        let old: Vec<Unit> = (0..400)
            .map(|i| unit(&format!("The client MUST {words} old{i}.")))
            .collect();
        let new: Vec<Unit> = (0..400)
            .map(|i| unit(&format!("The client MUST {words} new{i}.")))
            .collect();
        let started = std::time::Instant::now();
        let (pairs, old_left, new_left) = pair(&old, &new);
        assert_eq!(pairs.len() + old_left.len(), 400);
        assert_eq!(pairs.len() + new_left.len(), 400);
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
