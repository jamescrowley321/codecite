//! A grep baseline: ranks a pinned commit's files by how many of the
//! query's words they hold, as an agent with `git grep` and nothing else
//! could.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, bail};

/// Words too common to say anything about a file, including the
/// conventional-commit types that start most PR titles.
const STOPWORDS: &[&str] = &[
    "add", "and", "are", "but", "chore", "docs", "feat", "fix", "for", "from", "has", "have",
    "into", "its", "not", "our", "test", "that", "the", "this", "use", "was", "what", "when",
    "with",
];

/// The query's words: lowercased runs of letters, digits and `_`, of at
/// least 3 characters, without [`STOPWORDS`] or repeats, in order.
fn terms(text: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        let word = word.to_lowercase();
        if word.chars().count() >= 3
            && !STOPWORDS.contains(&word.as_str())
            && !terms.contains(&word)
        {
            terms.push(word);
        }
    }
    terms
}

/// The files of `commit` in the repository at `git_dir`, best first and at
/// most `depth`, each cited as `grep@<commit>:<path>#L1`.
///
/// A file scores the number of the query's words it holds as whole words,
/// in any case; ties go to more matching lines, then to the path.
pub fn grep_rank(
    git_dir: &Path,
    commit: &str,
    text: &str,
    depth: u32,
) -> anyhow::Result<Vec<String>> {
    // The commit is git's last argument, so anything but a hash could be
    // read as an option.
    if !(7..=64).contains(&commit.len()) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("`{commit}` is not a commit hash");
    }
    // path -> (distinct terms, matching lines)
    let mut scores: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    let prefix = format!("{commit}:");
    for term in terms(text) {
        let output = Command::new("git")
            .arg("-C")
            .arg(git_dir)
            .args(["grep", "-c", "-i", "-F", "-w", "-e", &term, commit])
            .output()
            .context("running git grep")?;
        // 1 is no match; anything else is a failure.
        match output.status.code() {
            Some(0) => {}
            Some(1) => continue,
            _ => bail!(
                "git grep failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Some(rest) = line.strip_prefix(&prefix) else {
                continue;
            };
            let Some((path, count)) = rest.rsplit_once(':') else {
                continue;
            };
            let count: u64 = count.parse().unwrap_or(0);
            let entry = scores.entry(path.to_owned()).or_default();
            entry.0 += 1;
            entry.1 += count;
        }
    }
    let mut ranked: Vec<(String, (usize, u64))> = scores.into_iter().collect();
    // The map is in path order, and the sort is stable.
    ranked.sort_by_key(|a| std::cmp::Reverse(a.1));
    Ok(ranked
        .into_iter()
        .take(depth as usize)
        .map(|(path, _)| format!("grep@{commit}:{path}#L1"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    /// Runs git in `dir` with a fixed identity; panics on failure.
    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=bench",
                "-c",
                "user.email=bench@example.com",
            ])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    /// A repository whose first commit holds `a.py` (token, refresh) and
    /// `b.py` (token); a second commit adds `c.py`. Returns the first.
    fn repo() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        std::fs::write(
            dir.path().join("a.py"),
            "def refresh(token):\n    return token\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.py"),
            "TOKEN = 1\ntoken = 2\nprint(token)\n",
        )
        .unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "one"]);
        let head = git(dir.path(), &["rev-parse", "HEAD"]).trim().to_owned();
        std::fs::write(dir.path().join("c.py"), "only_in_c = 1\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "two"]);
        (dir, head)
    }

    fn paths(citations: &[String]) -> Vec<String> {
        citations
            .iter()
            .map(|c| {
                c.split_once(':')
                    .unwrap()
                    .1
                    .split_once('#')
                    .unwrap()
                    .0
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn files_matching_more_query_words_rank_first() {
        let (dir, head) = repo();
        let ranked = grep_rank(dir.path(), &head, "Refresh the token", 10).unwrap();
        assert_eq!(paths(&ranked), ["a.py", "b.py"]);
        assert_eq!(ranked[0], format!("grep@{head}:a.py#L1"));
    }

    #[test]
    fn ties_break_on_total_matches() {
        let (dir, head) = repo();
        // Both hold "token"; b.py on three lines, a.py on two.
        assert_eq!(
            paths(&grep_rank(dir.path(), &head, "token", 10).unwrap()),
            ["b.py", "a.py"]
        );
    }

    #[test]
    fn a_query_with_no_matching_word_ranks_nothing() {
        let (dir, head) = repo();
        assert!(
            grep_rank(dir.path(), &head, "zzzz qqqq", 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            grep_rank(dir.path(), &head, "the and of", 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn query_syntax_is_searched_as_text() {
        let (dir, head) = repo();
        let ranked = grep_rank(dir.path(), &head, "fix \"token\" * \\ -e --all [a-z]", 10).unwrap();
        assert_eq!(paths(&ranked), ["b.py", "a.py"]);
    }

    #[test]
    fn ranks_the_pinned_commit_not_the_work_tree() {
        let (dir, head) = repo();
        assert!(
            grep_rank(dir.path(), &head, "only_in_c", 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn depth_caps_the_results() {
        let (dir, head) = repo();
        assert_eq!(grep_rank(dir.path(), &head, "token", 1).unwrap().len(), 1);
    }

    #[test]
    fn a_commit_that_is_not_a_hash_is_refused_before_git_runs() {
        let (dir, _) = repo();
        let err = grep_rank(dir.path(), "--open-files-in-pager=touch", "token", 10).unwrap_err();
        assert!(err.to_string().contains("not a commit hash"), "{err}");
    }
}
