"""Index the four non-Linux generic-retrieval repos with two codecite
binaries and compare every case's codecite def/search output and hit rank."""
import json, subprocess, sys, os
from pathlib import Path
sys.path.insert(0, sys.argv[1])  # experiments/retrieval-generalization
import run as R
here, root, src = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
bins = dict(main=sys.argv[4], branch=sys.argv[5])
repos = ['ripgrep', 'excalidraw', 'django', 'keycloak']
cases = [c for c in R.load_cases() if c['repo'] in repos]
corpora = R.load_corpora()
out = {}
for name, b in bins.items():
    run = root / name
    for d in ('data', 'cache', 'home', 'manifests'):
        (run / d).mkdir(parents=True, exist_ok=True)
    env = R.environment(run)
    for repo in repos:
        lic, use = corpora[repo]['license'], corpora[repo]['use']
        m = run / 'manifests' / f'{repo}.toml'
        m.write_text(f'name = "{repo}"\nmax_age_days = 365\n\n[[source]]\nkind = "local"\nname = "{repo}"\n'
                     f'path = "{src / repo}"\nlicense = "{lic}"\nuse = "{use}"\nexclude = [".codegraph/**"]\n')
        p = subprocess.run([b, 'index', repo, '--manifest', str(m)], env=env, capture_output=True, text=True)
        print(name, repo, p.returncode, [l for l in p.stdout.splitlines() if l.startswith('chunks')], file=sys.stderr)
        assert p.returncode == 0, p.stderr
    for c in cases:
        for op in ('def', 'search'):
            if op == 'def' and c['class'] not in R.DEFINITION_CLASSES:
                continue
            cmd = R.argv('codecite', op, c, b, str(src))
            proc, _ = R.call(cmd, env)
            st, rows, total = R.parse('codecite', op, proc.returncode, proc.stdout.decode(), proc.stderr.decode())
            rank = R.first_hit(rows, c['answers']) if c.get('score') and c['answers'] else None
            out.setdefault((c['id'], op), {})[name] = dict(status=st, rows=rows, rank=rank)
same = diff = lost = gained = 0
for k, v in sorted(out.items()):
    a, b2 = v['main'], v['branch']
    if a['rows'] == b2['rows'] and a['status'] == b2['status']:
        same += 1
    else:
        diff += 1
        print('DIFF', k, a['rank'], b2['rank'])
    if a['rank'] == 1 and b2['rank'] != 1: lost += 1; print('LOST@1', k, b2['rank'])
    if a['rank'] and not b2['rank']: lost += 1; print('LOST', k)
print(json.dumps(dict(calls=len(out), identical=same, differing=diff, losses=lost)))
