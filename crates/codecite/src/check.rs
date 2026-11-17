//! `codecite cite check`: find the citations written in files and check
//! each against a corpus, for a CI job to fail on any that don't hold up.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use codecite_core::{CheckOptions, CitationProblem, Problem, Store};
use serde_json::json;

/// A file larger than this is skipped: it is not text a person cites from.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// What a check of some files found.
#[derive(Debug, Default)]
pub struct Report {
    /// How many files were read.
    pub files: usize,
    /// How many citations they hold.
    pub citations: usize,
    /// Each problem, with the file it is in, in file order.
    pub problems: Vec<(PathBuf, CitationProblem)>,
}

/// Checks every citation in the files at `paths`, each a file or a
/// directory walked for files, against `store`.
pub fn check(store: &Store, paths: &[PathBuf], options: &CheckOptions) -> anyhow::Result<Report> {
    let mut checker = store.citation_checker(options)?;
    let mut report = Report::default();
    let mut files = Vec::new();
    for path in paths {
        collect(path, true, &mut files)?;
    }
    for file in files {
        let Some(text) = read_text(&file)? else {
            continue;
        };
        report.files += 1;
        let found = checker.check(&text)?;
        report.citations += found.citations;
        report
            .problems
            .extend(found.problems.into_iter().map(|p| (file.clone(), p)));
    }
    Ok(report)
}

/// Adds `path` to `files`, or, for a directory, every file under it in
/// name order. A link or hidden entry inside a directory is skipped; a
/// path named on the command line is taken as given.
fn collect(path: &Path, named: bool, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    let meta = if named {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    }
    .with_context(|| format!("reading {}", path.display()))?;
    if meta.is_file() {
        files.push(path.to_owned());
    } else if meta.is_dir() {
        let mut entries = fs::read_dir(path)
            .with_context(|| format!("reading {}", path.display()))?
            .collect::<io::Result<Vec<_>>>()
            .with_context(|| format!("reading {}", path.display()))?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            collect(&entry.path(), false, files)?;
        }
    }
    Ok(())
}

/// The file's text, or `None` when it is over [`MAX_FILE_BYTES`], not
/// UTF-8, or holds a NUL byte, as a binary file does.
fn read_text(path: &Path) -> anyhow::Result<Option<String>> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .with_context(|| format!("reading {}", path.display()))?;
    if bytes.len() as u64 > MAX_FILE_BYTES || bytes.contains(&0) {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes).ok())
}

/// Writes the report: as one JSON object, or one line per problem, then a
/// summary line. An unresolved citation's candidates are indented under
/// it; a refused one's message already lists them.
pub fn write_report(out: &mut impl Write, report: &Report, json: bool) -> io::Result<()> {
    if json {
        let problems: Vec<serde_json::Value> = report
            .problems
            .iter()
            .map(|(path, problem)| {
                let mut value = serde_json::to_value(problem).unwrap_or_default();
                value["path"] = json!(path.display().to_string());
                value
            })
            .collect();
        let value = json!({
            "files": report.files,
            "citations": report.citations,
            "failed": report.problems.len(),
            "problems": problems,
        });
        writeln!(out, "{value}")?;
        return out.flush();
    }
    for (path, p) in &report.problems {
        let line = format!(
            "{}:{}:{}: {} · {}",
            path.display(),
            p.line,
            p.column,
            p.citation,
            p.problem
        );
        writeln!(out, "{}", crate::escape_controls(&line))?;
        if let Problem::Unresolved { candidates, .. } = &p.problem {
            for candidate in candidates {
                writeln!(out, "  {}", crate::escape_controls(candidate))?;
            }
        }
    }
    writeln!(
        out,
        "{} in {}: {} failed",
        plural(report.citations, "citation"),
        plural(report.files, "file"),
        report.problems.len()
    )?;
    out.flush()
}

fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
