//! What a query costs an agent: the bytes `codecite search` prints and the
//! time it takes, next to `rg`'s on the same words over the same tree.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context as _, bail};
use serde::Serialize;

use crate::Query;
use crate::grep::terms;

/// How many times each command runs per query; the fastest run counts.
const RUNS: usize = 3;

/// What `rg` prints.
#[derive(Clone, Copy)]
pub enum RgMode {
    /// Every matching line, `path:line:text`: `rg -n`.
    Lines,
    /// Only the matching files' paths: `rg -l`.
    Files,
}

/// The median and 90th percentile of what an engine costs per query.
#[derive(Debug, Serialize)]
pub struct Cost {
    pub median_bytes: u64,
    pub median_ms: f64,
    pub p90_ms: f64,
}

impl Cost {
    /// The cost of a set of `(bytes, milliseconds)` measurements, one per
    /// query. Percentiles interpolate linearly between the nearest ranks.
    pub fn of(measured: &[(u64, f64)]) -> Cost {
        let mut bytes: Vec<f64> = measured.iter().map(|m| m.0 as f64).collect();
        let mut ms: Vec<f64> = measured.iter().map(|m| m.1).collect();
        bytes.sort_by(f64::total_cmp);
        ms.sort_by(f64::total_cmp);
        Cost {
            median_bytes: percentile(&bytes, 0.5).round() as u64,
            median_ms: percentile(&ms, 0.5),
            p90_ms: percentile(&ms, 0.9),
        }
    }
}

/// The `p` quantile of sorted values, 0 for none.
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let at = p * (sorted.len() - 1) as f64;
    let (low, high) = (at.floor() as usize, at.ceil() as usize);
    sorted[low] + (sorted[high] - sorted[low]) * (at - low as f64)
}

/// `rg`'s arguments, with no path so it searches the working directory and
/// prints repo-relative paths, as an agent at the repo root sees them: each term as a whole word in any case, as the `git
/// grep` baseline reads them.
pub fn rg_args(terms: &[String], mode: RgMode) -> Vec<String> {
    let mut args = vec![
        match mode {
            RgMode::Lines => "-n",
            RgMode::Files => "-l",
        }
        .to_owned(),
        "-i".into(),
        "-w".into(),
        "-F".into(),
    ];
    for term in terms {
        args.push("-e".into());
        args.push(term.clone());
    }
    args
}

/// The query's words, or `None` when it has none: neither engine is
/// measured on it.
pub fn measured_terms(text: &str) -> Option<Vec<String>> {
    Some(terms(text))
        .map(|t| {
            t.into_iter()
                .filter(|t| t.chars().any(char::is_alphanumeric))
                .collect::<Vec<_>>()
        })
        .filter(|t| !t.is_empty())
}

/// Runs `program`, returning what it printed to stdout in bytes and the
/// wall time in milliseconds. A failure names the program.
pub fn run_timed(program: &Path, args: &[String]) -> anyhow::Result<(u64, f64)> {
    run_accepting(program, args, &[0], None)
}

/// [`run_timed`], where exit codes in `ok` also count as success.
fn run_accepting(
    program: &Path,
    args: &[String],
    ok: &[i32],
    dir: Option<&Path>,
) -> anyhow::Result<(u64, f64)> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let start = Instant::now();
    let output = command
        .output()
        .with_context(|| format!("running {}", program.display()))?;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    if !output.status.code().is_some_and(|c| ok.contains(&c)) {
        bail!(
            "{} failed ({}): {}",
            program.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok((output.stdout.len() as u64, ms))
}

/// The fastest of [`RUNS`] runs. Bytes are the same every run.
pub fn fastest(run: impl Fn() -> anyhow::Result<(u64, f64)>) -> anyhow::Result<(u64, f64)> {
    let mut best: Option<(u64, f64)> = None;
    for _ in 0..RUNS {
        let run = run()?;
        if best.is_none_or(|b| run.1 < b.1) {
            best = Some(run);
        }
    }
    Ok(best.expect("RUNS is not 0"))
}

/// Measures `codecite search` (default limit, human output: what an agent
/// sees) and `rg -n` and `rg -l` over the checkout, for every query with
/// words in it. `rg` is the program to run; no match is 0 bytes.
pub fn cost(
    queries: &[&Query],
    corpus: &str,
    codecite: &Path,
    rg: &Path,
    checkout: &Path,
) -> anyhow::Result<serde_json::Value> {
    let (mut ours, mut lines, mut files) = (Vec::new(), Vec::new(), Vec::new());
    for query in queries {
        let Some(words) = measured_terms(&query.text) else {
            continue;
        };
        let context = || format!("query `{}`", query.id);
        let search = ["search", "--allow-stale", "--", corpus, &query.text].map(str::to_owned);
        // The bench's words can include one codecite drops; then no engine
        // is measured on the query.
        match fastest(|| run_timed(codecite, &search)) {
            Ok(m) => ours.push(m),
            Err(e) if crate::has_no_words(format!("{e:#}").as_bytes()) => continue,
            Err(e) => return Err(e).with_context(context),
        }
        for (mode, into) in [(RgMode::Lines, &mut lines), (RgMode::Files, &mut files)] {
            let args = rg_args(&words, mode);
            into.push(
                fastest(|| run_accepting(rg, &args, &[0, 1], Some(checkout)))
                    .with_context(context)?,
            );
        }
    }
    Ok(serde_json::json!({
        "engine": "cost",
        "queries": ours.len(),
        "codecite": Cost::of(&ours),
        "rg_lines": Cost::of(&lines),
        "rg_files": Cost::of(&files),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rg_args_search_each_term_as_a_whole_word_in_any_case() {
        assert_eq!(
            rg_args(&["token".into(), "refresh".into()], RgMode::Lines),
            ["-n", "-i", "-w", "-F", "-e", "token", "-e", "refresh"]
        );
        assert_eq!(rg_args(&["x_y".into()], RgMode::Files)[0], "-l");
    }

    #[test]
    fn medians_and_p90_of_measurements() {
        let c = Cost::of(&[(10, 1.0), (30, 3.0), (20, 2.0), (40, 4.0)]);
        assert_eq!((c.median_bytes, c.median_ms), (25, 2.5));
        assert!((c.p90_ms - 3.7).abs() < 1e-9);
    }

    #[test]
    fn a_query_with_no_words_is_not_measured() {
        assert!(measured_terms("a to of").is_none());
        assert!(measured_terms("token refresh").is_some());
        assert!(measured_terms("___ to").is_none());
    }

    #[test]
    fn a_missing_rg_is_named() {
        let err = run_timed(Path::new("rg-not-installed-here"), &[]).unwrap_err();
        assert!(err.to_string().contains("rg-not-installed-here"), "{err}");
    }

    #[test]
    fn a_command_given_a_directory_runs_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let script = ["-c".to_owned(), "pwd -P; ls".to_owned()];
        let (bytes, _) = run_accepting(Path::new("sh"), &script, &[0], Some(dir.path())).unwrap();
        let canonical = dir.path().canonicalize().unwrap().display().to_string();
        assert_eq!(bytes as usize, canonical.len() + 1);
    }

    #[test]
    fn no_match_is_zero_bytes_for_rg_but_a_failure_for_the_rest() {
        let sh = Path::new("sh");
        let exit1 = ["-c".to_owned(), "exit 1".to_owned()];
        assert_eq!(run_accepting(sh, &exit1, &[0, 1], None).unwrap().0, 0);
        assert!(run_timed(sh, &exit1).is_err());
    }
}
