//! The modules a source file imports, read line by line, so a rule scoped
//! to a dependency (`imports = ["requests"]`) can find the files it
//! governs. A line scan, not a parse: it reads the import forms each
//! language writes at the start of a line, and a string or comment that
//! looks like one can count too.

use std::sync::LazyLock;

use regex::Regex;

use crate::Language;

/// The modules `text`, a file in `language`, imports, in the order first
/// written and each once: `requests.adapters` for Python's
/// `from requests.adapters import HTTPAdapter`, `node:fs` for a
/// TypeScript `import fs from "node:fs"`, `std::collections` for a Rust
/// `use std::collections::HashMap;`. A relative import (`from . import x`,
/// `./util`, `crate::`, `self::`, `super::`) names no dependency and is
/// left out, and so is any file in a language with no import form.
pub fn imports(language: Language, text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut push = |module: &str| {
        let module = module.trim();
        if !module.is_empty()
            && !is_relative(language, module)
            && !found.iter().any(|m| m == module)
        {
            found.push(module.to_owned());
        }
    };
    match language {
        Language::Python => {
            for line in text.lines() {
                if let Some(c) = PY_FROM.captures(line) {
                    push(&c[1]);
                } else if let Some(c) = PY_IMPORT.captures(line) {
                    for part in c[1].split(',') {
                        // `import numpy as np` imports `numpy`.
                        push(part.split_whitespace().next().unwrap_or(""));
                    }
                }
            }
        }
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            for c in JS.captures_iter(text) {
                push(c.get(1).or(c.get(2)).map_or("", |m| m.as_str()));
            }
        }
        Language::Rust => {
            for line in text.lines() {
                if let Some(c) = RUST_USE.captures(line) {
                    let path = c[1].trim_start_matches("::");
                    // `use std::collections::{HashMap, HashSet};` imports
                    // from `std::collections`.
                    push(path.trim_end_matches("::"));
                }
            }
        }
        Language::Go => {
            let mut block = false;
            for line in text.lines() {
                let line = line.trim();
                if block {
                    if line.starts_with(')') {
                        block = false;
                    } else if let Some(c) = GO_SPEC.captures(line) {
                        push(&c[1]);
                    }
                } else if let Some(rest) = line.strip_prefix("import") {
                    let rest = rest.trim_start();
                    if let Some(inner) = rest.strip_prefix('(') {
                        // `import ("fmt"; "os")` opens and closes on one line.
                        let (specs, closed) = match inner.find(')') {
                            Some(end) => (&inner[..end], true),
                            None => (inner, false),
                        };
                        for spec in specs.split(';') {
                            if let Some(c) = GO_SPEC.captures(spec.trim()) {
                                push(&c[1]);
                            }
                        }
                        block = !closed;
                    } else if let Some(c) = GO_SPEC.captures(rest) {
                        push(&c[1]);
                    }
                }
            }
        }
        Language::CSharp => {
            for line in text.lines() {
                if let Some(c) = CS_USING.captures(line) {
                    push(&c[1]);
                }
            }
        }
        Language::Markdown => {}
    }
    found
}

/// Whether `module`, an import a file writes, is `dependency` or inside it:
/// `requests` covers `requests` and `requests.adapters`, `std` covers
/// `std::collections`, and `node:fs` only itself. Modules are compared
/// whole, part by part, so `request` does not cover `requests`.
pub fn imports_dependency(module: &str, dependency: &str) -> bool {
    module.strip_prefix(dependency).is_some_and(|rest| {
        rest.is_empty() || rest.starts_with(['.', '/']) || rest.starts_with("::")
    })
}

/// An import of the file's own package rather than a dependency: one that
/// starts with `.`, or in Rust one under `crate`, `self` or `super`.
fn is_relative(language: Language, module: &str) -> bool {
    module.starts_with('.')
        || (language == Language::Rust
            && ["crate", "self", "super"]
                .iter()
                .any(|word| imports_dependency(module, word)))
}

/// `from x.y import z`: the module is `x.y`. A relative `from .x` starts
/// with a dot.
static PY_FROM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*from\s+([\w.]+)\s+import\b").expect("valid regex"));

/// `import x, y.z as w`.
static PY_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*import\s+([\w.]+(?:\s+as\s+\w+)?(?:\s*,\s*[\w.]+(?:\s+as\s+\w+)?)*)")
        .expect("valid regex")
});

/// `import ... from "x"`, `export ... from "x"`, `import "x"`, `require("x")`
/// and `import("x")`.
static JS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?m)(?:^\s*(?:import|export)\b[^'";]*?\bfrom\s*|^\s*import\s*)['"]([^'"\n]+)['"]|\b(?:require|import)\s*\(\s*['"]([^'"\n]+)['"]\s*\)"#,
    )
    .expect("valid regex")
});

/// `use a::b::c;`, `pub use a::b::{c, d};`, `extern crate a;`. The path
/// stops before the last name or a brace.
static RUST_USE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:use|extern\s+crate)\s+((?:::)?\w+(?:::\w+)*::|\w+)",
    )
    .expect("valid regex")
});

/// One Go import spec: `"net/http"`, `h "net/http"`, `_ "embed"`.
static GO_SPEC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^(?:[\w.]+\s+)?"([^"]+)""#).expect("valid regex"));

/// `using System.Net.Http;`, `global using static X;`, `using A = B.C;`.
static CS_USING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:global\s+)?using\s+(?:static\s+)?(?:\w+\s*=\s*)?([\w.]+)\s*;")
        .expect("valid regex")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_imports() {
        let text = "import os\nimport numpy as np, requests.adapters\nfrom httpx import Client\n\
                    from . import local\nfrom .sibling import x\n  import json  # indented\nx = 'import nope'\n";
        assert_eq!(
            imports(Language::Python, text),
            ["os", "numpy", "requests.adapters", "httpx", "json"]
        );
    }

    #[test]
    fn javascript_and_typescript_imports() {
        let text = "import fs from \"node:fs\";\nimport { a,\n  b } from 'lodash';\nimport './side-effect.css';\n\
                    import 'polyfill';\nexport * from \"zod\";\nconst x = require('axios');\n\
                    const y = await import(\"./lazy\");\nconst s = 'from \"nope\"';\n";
        for language in [Language::TypeScript, Language::Tsx, Language::JavaScript] {
            assert_eq!(
                imports(language, text),
                ["node:fs", "lodash", "polyfill", "zod", "axios"],
                "{language:?}"
            );
        }
    }

    #[test]
    fn a_multi_line_import_reads_its_module() {
        let text = "import {\n  a,\n  b,\n} from \"lodash\";\n";
        assert_eq!(imports(Language::TypeScript, text), ["lodash"]);
    }

    #[test]
    fn rust_uses() {
        let text = "use std::collections::{HashMap, HashSet};\npub use serde::Serialize;\n\
                    pub(crate) use reqwest;\nuse crate::Error;\nuse super::x;\nextern crate alloc;\nuse ::regex::Regex;\n";
        assert_eq!(
            imports(Language::Rust, text),
            ["std::collections", "serde", "reqwest", "alloc", "regex"]
        );
    }

    #[test]
    fn go_imports() {
        let text = "package main\n\nimport \"fmt\"\nimport (\n\t\"net/http\"\n\th \"github.com/a/b\"\n\t_ \"embed\"\n)\n";
        assert_eq!(
            imports(Language::Go, text),
            ["fmt", "net/http", "github.com/a/b", "embed"]
        );
    }

    #[test]
    fn a_go_import_block_on_one_line_closes() {
        let text = "import (\"fmt\"; h \"net/http\")\nfunc f() string {\n\treturn \"ok\"\n}\n";
        assert_eq!(imports(Language::Go, text), ["fmt", "net/http"]);
        assert!(imports(Language::Go, "import ()\nx \"y\"\n").is_empty());
    }

    #[test]
    fn only_rust_has_crate_self_and_super() {
        assert_eq!(
            imports(Language::Python, "from crate import client\n"),
            ["crate"]
        );
        assert_eq!(
            imports(Language::JavaScript, "import x from \"self\";\n"),
            ["self"]
        );
        assert!(imports(Language::Rust, "use crate::x;\n").is_empty());
    }

    #[test]
    fn csharp_usings() {
        let text = "using System;\nglobal using static System.Math;\nusing Http = System.Net.Http;\nusing (var x = y) {}\n";
        assert_eq!(
            imports(Language::CSharp, text),
            ["System", "System.Math", "System.Net.Http"]
        );
    }

    #[test]
    fn markdown_imports_nothing() {
        assert!(imports(Language::Markdown, "import os\n").is_empty());
    }

    #[test]
    fn a_dependency_covers_its_submodules_only() {
        assert!(imports_dependency("requests", "requests"));
        assert!(imports_dependency("requests.adapters", "requests"));
        assert!(imports_dependency("github.com/a/b/c", "github.com/a/b"));
        assert!(imports_dependency("std::collections", "std"));
        assert!(!imports_dependency("node:fs", "node"));
        assert!(imports_dependency("node:fs", "node:fs"));
        assert!(!imports_dependency("requests_oauthlib", "requests"));
        assert!(!imports_dependency("request", "requests"));
        assert!(!imports_dependency("requests", "request"));
    }
}
