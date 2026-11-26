"""Count the tree-sitter definitions two codecite binaries index (#312).

    python3 counts.py <workdir> <base-binary> <branch-binary> <generic-src>

Indexes, with each binary, the history benchmark's repositories at their
pinned commits (extracted from the clones in ~/.cache/codecite/history) and
the generic-retrieval repositories in <generic-src> (ripgrep, excalidraw,
django, keycloak), each as a `local` source, then counts `symbol` rows by
language and kind. Writes results/counts.md beside this script.
"""
import collections, os, re, sqlite3, subprocess, sys
from pathlib import Path

here = Path(__file__).resolve().parent
root = here.parents[1]
work, generic = Path(sys.argv[1]), Path(sys.argv[4])
bins = {'base': sys.argv[2], 'branch': sys.argv[3]}
clones = Path.home() / '.cache/codecite/history'

repos = {}
for manifest in sorted((root / 'evals/history').glob('*/manifest.toml')):
    commit = re.search(r'^commit = "([0-9a-f]+)"', manifest.read_text(), re.M).group(1)
    src = work / 'src' / manifest.parent.name
    if not src.exists():
        src.mkdir(parents=True)
        archive = subprocess.run(['git', '-C', clones / manifest.parent.name, 'archive', commit],
                                 check=True, capture_output=True).stdout
        subprocess.run(['tar', '-x', '-C', src], input=archive, check=True)
        # A `local` source must be a git checkout.
        git = ['git', '-C', src, '-c', 'user.name=counts', '-c', 'user.email=counts@localhost']
        subprocess.run(['git', 'init', '-q', src], check=True)
        subprocess.run([*git, 'add', '-A'], check=True)
        subprocess.run([*git, 'commit', '-qm', commit], check=True)
    repos[manifest.parent.name] = src
for name in ('ripgrep', 'excalidraw', 'django', 'keycloak'):
    repos[name] = generic / name

counts = {}
for label, binary in bins.items():
    run = work / label
    for d in ('data', 'cache', 'home', 'manifests'):
        (run / d).mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, XDG_DATA_HOME=str(run / 'data'), XDG_CACHE_HOME=str(run / 'cache'),
               HOME=str(run / 'home'), NO_COLOR='1')
    for repo, src in repos.items():
        manifest = run / 'manifests' / f'{repo}.toml'
        manifest.write_text(f'name = "{repo}"\n\n[[source]]\nkind = "local"\nname = "{repo}"\n'
                            f'path = "{src}"\nuse = "adapt"\nexclude = [".git/**", ".codegraph/**"]\n')
        subprocess.run([binary, 'index', repo, '--manifest', str(manifest)], env=env,
                       check=True, capture_output=True)
        db = sqlite3.connect(run / 'data/codecite' / f'{repo}.db')
        counts[label, repo] = collections.Counter({
            (lang or '-', kind or '-'): n for lang, kind, n in db.execute(
                'SELECT d.language, s.kind, count(*) FROM symbol s '
                'JOIN document d ON d.id = s.document_id GROUP BY 1, 2')})
        db.close()

lines = ['# Tree-sitter definitions, base vs branch (#312)', '',
         'Symbols by language and kind in each index; only rows that changed. '
         'Made by `counts.py`.', '']
for repo in repos:
    base, branch = counts['base', repo], counts['branch', repo]
    keys = sorted(k for k in base.keys() | branch.keys() if base[k] != branch[k])
    lines += [f'## {repo}', '',
              f'Total: {sum(base.values())} → {sum(branch.values())}', '']
    if not keys:
        lines += ['No change.', '']
        continue
    lines += ['| language | kind | base | branch | change |', '| --- | --- | ---: | ---: | ---: |']
    lines += [f'| {l} | {k} | {base[l, k]} | {branch[l, k]} | {branch[l, k] - base[l, k]:+d} |'
              for l, k in keys]
    lines.append('')
(here / 'results/counts.md').write_text('\n'.join(lines))
print('\n'.join(lines))
