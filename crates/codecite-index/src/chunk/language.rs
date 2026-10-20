//! Language detection by file extension, then by shebang.

/// A language the chunker recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Language {
    /// Rust (`.rs`).
    Rust,
    /// Python (`.py`, `.pyi`, or a `python` shebang).
    Python,
    /// TypeScript (`.ts`, `.mts`, `.cts`, or a `ts-node` shebang).
    TypeScript,
    /// TypeScript with JSX (`.tsx`).
    Tsx,
    /// JavaScript, JSX included (`.js`, `.mjs`, `.cjs`, `.jsx`, or a `node` shebang).
    JavaScript,
    /// Go (`.go`).
    Go,
    /// C# (`.cs`).
    CSharp,
    /// Markdown (`.md`, `.markdown`).
    Markdown,
}

impl Language {
    /// Detects the language of `path` from its extension, or, for a file
    /// with no known extension, from the shebang on the first line of `text`.
    pub fn detect(path: &str, text: &str) -> Option<Self> {
        Self::from_extension(path).or_else(|| Self::from_shebang(text))
    }

    /// The name stored in `document.language`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Go => "go",
            Self::CSharp => "csharp",
            Self::Markdown => "markdown",
        }
    }

    fn from_extension(path: &str) -> Option<Self> {
        let file = path.rsplit('/').next().unwrap_or(path);
        let (stem, ext) = file.rsplit_once('.')?;
        if stem.is_empty() {
            return None; // `.rs` alone is a hidden file, not a Rust file
        }
        Some(match ext.to_ascii_lowercase().as_str() {
            "rs" => Self::Rust,
            "py" | "pyi" => Self::Python,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "go" => Self::Go,
            "cs" => Self::CSharp,
            "md" | "markdown" => Self::Markdown,
            _ => return None,
        })
    }

    fn from_shebang(text: &str) -> Option<Self> {
        let line = text.lines().next()?.strip_prefix("#!")?;
        let mut words = line.split_whitespace();
        let mut program = words.next()?.rsplit('/').next()?;
        if program == "env" {
            // `#!/usr/bin/env -S node --flag`: the first word that isn't a flag.
            program = words.find(|w| !w.starts_with('-'))?;
        }
        match program {
            "node" | "nodejs" => Some(Self::JavaScript),
            "ts-node" => Some(Self::TypeScript),
            p if p.starts_with("python") => Some(Self::Python),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_pick_the_language() {
        let cases = [
            ("src/lib.rs", Language::Rust),
            ("app/main.py", Language::Python),
            ("types.pyi", Language::Python),
            ("src/index.ts", Language::TypeScript),
            ("src/util.mts", Language::TypeScript),
            ("src/App.tsx", Language::Tsx),
            ("lib/a.js", Language::JavaScript),
            ("lib/a.cjs", Language::JavaScript),
            ("lib/View.jsx", Language::JavaScript),
            ("cmd/main.go", Language::Go),
            ("Client.cs", Language::CSharp),
            ("README.md", Language::Markdown),
            ("docs/Guide.MARKDOWN", Language::Markdown),
        ];
        for (path, language) in cases {
            assert_eq!(Language::detect(path, ""), Some(language), "{path}");
        }
    }

    #[test]
    fn unknown_or_hidden_files_have_no_language() {
        for path in [
            "notes.txt",
            "Makefile",
            ".rs",
            "dir.rs/README",
            "archive.tar.gz",
        ] {
            assert_eq!(Language::detect(path, "fn main() {}"), None, "{path}");
        }
    }

    #[test]
    fn shebang_names_the_language_of_a_file_without_an_extension() {
        let cases = [
            ("#!/usr/bin/env python3\nprint(1)\n", Some(Language::Python)),
            ("#!/usr/bin/python3.12\n", Some(Language::Python)),
            ("#!/usr/bin/env node\n", Some(Language::JavaScript)),
            (
                "#!/usr/bin/env -S node --no-warnings\n",
                Some(Language::JavaScript),
            ),
            ("#!/usr/local/bin/ts-node\n", Some(Language::TypeScript)),
            ("#!/bin/sh\necho hi\n", None),
            ("#!/usr/bin/env\n", None),
            ("print(1)\n#!/usr/bin/env python3\n", None),
        ];
        for (text, language) in cases {
            assert_eq!(Language::detect("bin/tool", text), language, "{text:?}");
        }
    }

    #[test]
    fn extension_wins_over_shebang() {
        assert_eq!(
            Language::detect("build.rs", "#!/usr/bin/env python3\n"),
            Some(Language::Rust)
        );
    }
}
