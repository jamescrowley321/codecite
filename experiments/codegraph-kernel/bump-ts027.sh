#!/usr/bin/env bash
# Port the fork's kernel from tree-sitter 0.25 to codecite's 0.27, in place.
# Run from a checkout of the fork's local/rlib-extraction branch (README,
# "Fork changes"). Every edit is mechanical: 0.27 takes a u32 child index,
# its progress callback returns ControlFlow, and one helper's lifetime
# must be spelled out.
set -euo pipefail
cd "${1:-.}/codegraph-kernel"
sed -i 's/^tree-sitter = "0.25"/tree-sitter = "0.27"/' Cargo.toml
grep -rlE '\.named_child\(' src | xargs perl -pi -e 's/\.named_child\(([^()]*(?:\([^()]*\))?[^()]*)\)/.named_child(($1) as u32)/g'
perl -pi -e 's/(\w+)\.named_child\(\1\.named_child_count\(\)\.checked_sub\(1\)\?\)/$1.named_child((($1.named_child_count().checked_sub(1)?) as u32))/g; s/(\w+)\.named_child\(\1\.named_child_count\(\)\.saturating_sub\(1\)\)/$1.named_child(($1.named_child_count().saturating_sub(1)) as u32)/g' src/*.rs src/*/*.rs
perl -0pi -e 's/let mut cancel = \|_: &ParseState\| Instant::now\(\) >= deadline;/let mut cancel = |_: &ParseState| if Instant::now() >= deadline { std::ops::ControlFlow::Break(()) } else { std::ops::ControlFlow::Continue(()) };/' src/parse_budget.rs
perl -pi -e "s/^fn macro_group_delim\(group: Node\) -> &'static str \{/fn macro_group_delim<'t>(group: Node<'t>) -> &'t str {/" src/rustlang.rs
