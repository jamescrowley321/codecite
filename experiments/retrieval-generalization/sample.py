#!/usr/bin/env python3
"""Draw candidate definitions for labelling, from source alone (#287).

    python3 -I sample.py <checkout> <repo> [--n 24] [--seed 287]

Reads only `git ls-files` and file text at the checkout's HEAD; it never runs
codecite or CodeGraph, so labels drawn from it cannot be tuned to either
tool's output. Files are drawn with a seeded RNG from the repository's
implementation files (paths that look like tests, docs, examples, fixtures
or vendored code are left out of the *workload*; this is a sampling choice,
not a ranking rule). One definition is drawn per file. `defs` counts how many
lines across the repository define the same name with the same patterns, so
a name with defs == 1 can be an exact case and a name with defs > 1 a
qualified/ambiguous case. The definition patterns are crude on purpose: a
label is kept only after a person reads the line it points at.
"""

import argparse
import json
import random
import re
import subprocess
from pathlib import Path

PATTERNS = {
    '.py': [r'^\s*(?:async\s+)?def\s+([A-Za-z_]\w*)\s*\(', r'^\s*class\s+([A-Za-z_]\w*)\b'],
    '.rs': [r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z_]\w*)',
            r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|trait)\s+([A-Za-z_]\w*)'],
    '.ts': [r'^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][\w$]*)',
            r'^\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?class\s+([A-Za-z_$][\w$]*)',
            r'^\s*(?:export\s+)?const\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_$][\w$]*)\s*(?::[^=]*)?=>'],
    '.java': [r'^\s*(?:public|protected|private)?\s*(?:abstract\s+|final\s+|static\s+)*(?:class|interface|enum|record)\s+([A-Za-z_]\w*)',
              r'^\s*(?:public|protected|private)\s+(?:static\s+|final\s+|abstract\s+|synchronized\s+)*[\w<>\[\],\s?]+\s+([a-z]\w*)\s*\([^;]*$'],
    '.c': [r'^(?:static\s+|inline\s+|__\w+\s+)*[A-Za-z_][\w\s\*]*?\b([A-Za-z_]\w*)\s*\([^;]*$', r'^struct\s+([A-Za-z_]\w*)\s*\{'],
}
PATTERNS['.tsx'] = PATTERNS['.ts']
PATTERNS['.h'] = PATTERNS['.c']
SKIP = re.compile(r'(^|/)(tests?|testing|__tests__|spec|specs|fixtures?|testdata|docs?|examples?|samples?|vendor|third_party|node_modules|benches|scripts|tools|Documentation|samples)(/|$)'
                  r'|(^|/)test_[^/]*$|_test\.[a-z]+$|\.test\.[a-z]+$|\.spec\.[a-z]+$|Tests?\.java$')
KEYWORDS = {'if', 'for', 'while', 'switch', 'return', 'sizeof', 'main', 'new', 'init', 'self'}


def git(checkout, *args):
    return subprocess.check_output(['git', '-C', checkout, *args], text=True)


def definitions(text, ext):
    for number, line in enumerate(text.splitlines(), 1):
        for pattern in PATTERNS[ext]:
            match = re.match(pattern, line)
            if match:
                yield number, match.group(1)
                break


def context(lines, number):
    """The definition line, up to 8 lines before it and 12 after, for a person to read."""
    lo, hi = max(0, number - 9), min(len(lines), number + 12)
    return [f'{i + 1}: {lines[i][:160]}' for i in range(lo, hi)]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('checkout')
    parser.add_argument('repo')
    parser.add_argument('--n', type=int, default=24)
    parser.add_argument('--seed', type=int, default=287)
    args = parser.parse_args()
    files = [p for p in git(args.checkout, 'ls-files').splitlines()
             if Path(p).suffix in PATTERNS and not SKIP.search(p)]
    counts = {}
    texts = {}
    for path in files:
        try:
            text = (Path(args.checkout) / path).read_text(errors='replace')
        except OSError:
            continue
        texts[path] = text
        for _, name in definitions(text, Path(path).suffix):
            counts[name] = counts.get(name, 0) + 1
    rng = random.Random(f'{args.seed}:{args.repo}')
    order = sorted(texts)
    rng.shuffle(order)
    out = []
    for path in order:
        defs = [(n, name) for n, name in definitions(texts[path], Path(path).suffix)
                if len(name) >= 4 and not name.startswith('__') and name.lower() not in KEYWORDS]
        if not defs:
            continue
        number, name = rng.choice(defs)
        lines = texts[path].splitlines()
        out.append(dict(path=path, line=number, name=name, defs=counts[name],
                        context=context(lines, number)))
        if len(out) >= args.n:
            break
    print(json.dumps(dict(repo=args.repo, head=git(args.checkout, 'rev-parse', 'HEAD').strip(),
                          seed=args.seed, files=len(texts), samples=out), indent=1))


if __name__ == '__main__':
    main()
