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
    /// Each file found in a directory walk and not read, and why.
    pub skipped: Vec<(PathBuf, Skip)>,
}

/// Why a file was not read as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// It is over [`MAX_FILE_BYTES`].
    TooLarge,
    /// It is not UTF-8.
    NotUtf8,
    /// It holds a NUL byte, as a binary file does.
    NulByte,
}

impl Skip {
    const ALL: [Skip; 3] = [Skip::TooLarge, Skip::NotUtf8, Skip::NulByte];

    /// The reason's name in JSON.
    fn code(self) -> &'static str {
        match self {
            Skip::TooLarge => "over_1_mib",
            Skip::NotUtf8 => "not_utf8",
            Skip::NulByte => "nul_byte",
        }
    }

    /// The reason in words, after "is" or a count.
    fn words(self) -> &'static str {
        match self {
            Skip::TooLarge => "over 1 MiB",
            Skip::NotUtf8 => "not UTF-8",
            Skip::NulByte => "holding a NUL byte",
        }
    }
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
    let files = dedup(files);
    for (file, named) in files {
        let text = match read_text(&file)? {
            Ok(text) => text,
            // A file asked for by name that can't be checked fails the run;
            // one found in a walk is counted and reported.
            Err(skip) if named => {
                anyhow::bail!(
                    "{} is {}, so it can't be checked",
                    file.display(),
                    skip.words()
                )
            }
            Err(skip) => {
                report.skipped.push((file, skip));
                continue;
            }
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

/// `files` with each file once, in the order first found: a file named
/// twice, or inside a directory also named, is read once, and is named when
/// any of its entries was.
fn dedup(files: Vec<(PathBuf, bool)>) -> Vec<(PathBuf, bool)> {
    let mut first: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();
    let mut kept: Vec<(PathBuf, bool)> = Vec::new();
    for (file, named) in files {
        let key = fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
        match first.get(&key) {
            Some(&index) => kept[index].1 |= named,
            None => {
                first.insert(key, kept.len());
                kept.push((file, named));
            }
        }
    }
    kept
}

/// The paths in `list` (`-` for stdin), separated by NUL bytes, as
/// `git ls-files -z` prints them. Empty entries are left out.
pub fn read_paths0(list: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut bytes = Vec::new();
    if list == Path::new("-") {
        io::stdin()
            .lock()
            .read_to_end(&mut bytes)
            .context("reading paths from stdin")?;
    } else {
        bytes = fs::read(list).with_context(|| format!("reading {}", list.display()))?;
    }
    bytes
        .split(|&b| b == 0)
        .filter(|entry| !entry.is_empty())
        .map(path_from_bytes)
        .collect()
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> anyhow::Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt as _;
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> anyhow::Result<PathBuf> {
    let path = std::str::from_utf8(bytes).context("a listed path is not UTF-8")?;
    Ok(PathBuf::from(path))
}

/// Adds `path` to `files`, or, for a directory, every file under it in
/// name order, each with whether it was named. A link or hidden entry
/// inside a directory is skipped; a path named on the command line is taken
/// as given, and must be a file or a directory.
fn collect(path: &Path, named: bool, files: &mut Vec<(PathBuf, bool)>) -> anyhow::Result<()> {
    let meta = if named {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    }
    .with_context(|| format!("reading {}", path.display()))?;
    if meta.is_file() {
        files.push((path.to_owned(), named));
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
    } else if named {
        anyhow::bail!("{} is not a file or directory", path.display());
    }
    Ok(())
}

/// The file's text, or why it is not text: over [`MAX_FILE_BYTES`], not
/// UTF-8, or holding a NUL byte, as a binary file does.
fn read_text(path: &Path) -> anyhow::Result<Result<String, Skip>> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .with_context(|| format!("reading {}", path.display()))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Ok(Err(Skip::TooLarge));
    }
    if bytes.contains(&0) {
        return Ok(Err(Skip::NulByte));
    }
    Ok(String::from_utf8(bytes).map_err(|_| Skip::NotUtf8))
}

/// The path as a report line starts with it: a path that starts with
/// whitespace or `:` gets `./`, so CI does not take the line as a workflow
/// command such as `::error::`.
fn shown(path: &Path) -> String {
    let shown = path.display().to_string();
    if shown.starts_with(|c: char| c.is_whitespace() || c == ':') {
        format!("./{shown}")
    } else {
        shown
    }
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
        let skipped: Vec<serde_json::Value> = report
            .skipped
            .iter()
            .map(|(path, skip)| json!({"path": path.display().to_string(), "reason": skip.code()}))
            .collect();
        let value = json!({
            "files": report.files,
            "citations": report.citations,
            "failed": report.problems.len(),
            "problems": problems,
            "skipped": skipped,
        });
        writeln!(out, "{value}")?;
        return out.flush();
    }
    for (path, p) in &report.problems {
        let line = format!(
            "{}:{}:{}: {} · {}",
            shown(path),
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
    write!(
        out,
        "{} in {}: {} failed",
        plural(report.citations, "citation"),
        plural(report.files, "file"),
        report.problems.len()
    )?;
    if !report.skipped.is_empty() {
        let reasons: Vec<String> = Skip::ALL
            .iter()
            .filter_map(|&reason| {
                let n = report.skipped.iter().filter(|(_, s)| *s == reason).count();
                (n > 0).then(|| format!("{n} {}", reason.words()))
            })
            .collect();
        write!(
            out,
            "; {} skipped ({})",
            plural(report.skipped.len(), "file"),
            reasons.join(", ")
        )?;
    }
    writeln!(out)?;
    out.flush()
}

fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
