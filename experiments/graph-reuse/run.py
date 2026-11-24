#!/usr/bin/env python3
"""Run, score and report the CodeGraph reuse benchmark (#295). See README.

    python3 -I run.py validate --src SRC
    python3 -I run.py measure  --run RUN --src SRC --codecite BIN [--rounds 5]
    python3 -I run.py check    --run RUN --src SRC --codecite BIN
    python3 -I run.py report   --run RUN --src SRC

Every measured call is one local process under `unshare -rn` (no network):
the `codecite` or `codegraph` CLI, or `normalize.py` over a private CodeGraph
snapshot. No model, agent or MCP server is involved.

Arms, per case:

- `codecite`: the tree-sitter-only corpus, asked by name;
- `codecite+scip/name`: the same source with its SCIP index, asked by name;
- `codecite+scip/id`: the same, asked by the SCIP ID whose definition holds the
  case's line (found with an untimed `codecite def`);
- `codegraph/cli`: the `codegraph` CLI, asked by name (`Type::member` for a
  qualified case, CodeGraph's own syntax);
- `codegraph/normalized`: `normalize.py`, asked by path and line, every edge;
- `codegraph/normalized-trusted`: the same output, keeping only edges that are
  source-verified and have a stored confidence of at least 0.7.

A repository with no SCIP index gets no codecite+scip arms: its coverage is
`unavailable`, which is never scored as a miss.
"""

import argparse
import hashlib
import json
import os
import platform
import random
import re
import shutil
import statistics
import subprocess
import sys
import time
import tomllib
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
CLASSES = {'direct_call', 'non_call_reference', 'identical_name', 'qualified', 'indirect', 'unknown', 'miss'}
OPS = {'callers', 'callees', 'refs'}
CERTAINTIES = {'certain', 'ambiguous', 'indirect', 'unknown', 'none'}
ARMS = ['codecite', 'codecite+scip/name', 'codecite+scip/id', 'codegraph/cli', 'codegraph/normalized',
        'codegraph/normalized-trusted']
LIMIT = 50
START_WINDOW = 12
CITATION = re.compile(r'^[^@\s]+@[0-9a-f]+:(.+?)#L(\d+)(?:-L(\d+))?$')
WORD = r'(?<![A-Za-z0-9_$]){}(?![A-Za-z0-9_$])'


# ---------------------------------------------------------------------------
# Inputs


def load_corpora(path=HERE / 'corpora.toml'):
    data = tomllib.loads(Path(path).read_text())
    return {r['name']: r for r in data['repo']}, data['producer']


def load_cases(path=HERE / 'cases.json'):
    return json.loads(Path(path).read_text())['cases']


def git(checkout, *args):
    return subprocess.run(['git', '-C', str(checkout), *args], capture_output=True, text=True, check=False).stdout.strip()


def validate_cases(cases, repos, src):
    """Errors that make the case set unusable, found before any CLI runs."""
    errors = []
    seen = set()
    texts = {}
    checked = {}

    def line_of(repo, path, number):
        key = (repo, path)
        if key not in texts:
            try:
                texts[key] = (Path(src) / repo / path).read_text(errors='replace').splitlines()
            except OSError:
                texts[key] = None
        lines = texts[key]
        if lines is None or not 1 <= number <= len(lines):
            return None
        return lines[number - 1]

    def names(cid, repo, path, number, name, what):
        text = line_of(repo, path, number)
        if text is None:
            errors.append(f'{cid}: {what} {path}:{number} is a missing line')
        elif name and not re.search(WORD.format(re.escape(name)), text):
            errors.append(f'{cid}: {what} {path}:{number} does not name `{name}`')

    for case in cases:
        cid = case.get('id', '?')
        if cid in seen:
            errors.append(f'{cid}: duplicate id')
        seen.add(cid)
        repo = case.get('repo')
        if repo not in repos:
            errors.append(f'{cid}: unknown repository {repo}')
            continue
        if case.get('class') not in CLASSES:
            errors.append(f'{cid}: unknown class {case.get("class")}')
        if case.get('op') not in OPS:
            errors.append(f'{cid}: unknown op {case.get("op")}')
        if case.get('certainty') not in CERTAINTIES:
            errors.append(f'{cid}: unknown certainty {case.get("certainty")}')
        if repo not in checked:
            checkout = Path(src) / repo
            head = git(checkout, 'rev-parse', 'HEAD')
            problems = []
            if head != repos[repo]['commit']:
                problems.append(f'{repo}: checkout is not at {repos[repo]["commit"]}')
            elif git(checkout, 'status', '--porcelain', '--untracked-files=all'):
                problems.append(f'{repo}: checkout is not clean')
            checked[repo] = problems
            errors.extend(problems)
        if checked[repo]:
            continue
        sym = case['symbol']
        names(cid, repo, sym['path'], sym['line'], sym['name'], 'symbol')
        expect = case.get('expect', [])
        if case.get('class') in ('miss', 'unknown') and expect:
            errors.append(f'{cid}: a {case["class"]} case has answers (miss case or unknown site must expect none)')
        if case.get('class') not in ('miss', 'unknown') and not expect:
            errors.append(f'{cid}: a scored case has no answers')
        if case.get('site') is not None:
            names(cid, repo, sym['path'], case['site'], case.get('call_name'), 'site')
        for entry in expect:
            for target in entry.get('targets', []):
                if 'within' in target:
                    lo, hi = target['within']
                    if line_of(repo, target['path'], lo) is None or line_of(repo, target['path'], hi) is None:
                        errors.append(f'{cid}: target {target["path"]}:{lo}-{hi} is a missing line')
                else:
                    names(cid, repo, target['path'], target['line'], target['name'], 'target')
            if 'line_ref' in entry:
                ref = entry['line_ref']
                names(cid, repo, ref['path'], ref['line'], sym['name'], 'reference')
            if case.get('op') == 'callers':
                for target in entry.get('targets', []):
                    for site in entry.get('sites', []):
                        names(cid, repo, target['path'], site, sym['name'], 'site')
            elif case.get('op') == 'callees' and case.get('site') is None:
                for site in entry.get('sites', []):
                    called = [t['name'] for t in entry.get('targets', []) if 'name' in t]
                    text = line_of(repo, sym['path'], site)
                    if text is None:
                        errors.append(f'{cid}: site {sym["path"]}:{site} is a missing line')
                    elif called and not any(re.search(WORD.format(re.escape(n)), text) for n in called):
                        errors.append(f'{cid}: site {sym["path"]}:{site} does not name `{called[0]}`')
        for target in case.get('forbid', []):
            names(cid, repo, target['path'], target['line'], target['name'], 'forbidden target')
    return errors


# ---------------------------------------------------------------------------
# Output parsing


def codecite_status(code, out, err):
    if code == 0:
        return 'ok'
    if 'no SCIP index covers' in err:
        return 'unsupported'
    if re.search(r'names \d+ symbols|is ambiguous', err):
        return 'ambiguous'
    if 'defines no symbol' in err:
        return 'not_found'
    return 'error'


def codegraph_status(code, out, err):
    if code != 0:
        return 'error'
    if re.search(r'Symbol ".*" not found', out + err):
        return 'not_found'
    try:
        json.loads(out)
    except ValueError:
        return 'error'
    return 'ok'


def parse_citation(citation):
    match = CITATION.match(citation or '')
    if not match:
        return None
    path, start, end = match.group(1), int(match.group(2)), match.group(3)
    return path, start, int(end) if end else start


def parse_codecite(out, op):
    data = json.loads(out)
    got = []
    for row in data.get('results', []):
        if op == 'refs':
            cited = parse_citation(row.get('citation'))
            if cited:
                got.append({'path': cited[0], 'line': cited[1]})
            continue
        cited = parse_citation(row.get('citation'))
        site = parse_citation(row.get('call_site'))
        entry = {'path': cited[0] if cited else None, 'line': cited[1] if cited else None,
                 'end': cited[2] if cited else None, 'name': row.get('name'),
                 'site': site[1] if site else None, 'confidence': row.get('confidence')}
        got.append(entry)
    return got


def parse_codegraph(out, op):
    data = json.loads(out)
    rows = data.get(op, []) if isinstance(data, dict) else []
    return [{'path': r.get('filePath'), 'line': r.get('startLine'), 'end': None, 'name': r.get('name'), 'site': None}
            for r in rows]


def parse_normalized(out, op, trusted=False):
    data = json.loads(out)
    got = []
    for edge in data.get('edges', []):
        if trusted and (edge['evidence'] != 'verified' or edge['certainty'] != 'extracted'):
            continue
        if op == 'refs':
            if edge['site'] is not None:
                got.append({'path': edge['site']['path'], 'line': edge['site']['line']})
            continue
        end = edge['source'] if op == 'callers' else edge['target']
        if end is None:
            continue
        got.append({'path': end['path'], 'line': end['span'][0], 'end': end['span'][1], 'name': end['name'],
                    'site': edge['site']['line'] if edge['site'] else None, 'certainty': edge['certainty'],
                    'evidence': edge['evidence'], 'id': end['id']})
    return got, data.get('unresolved', [])


# ---------------------------------------------------------------------------
# Scoring


def endpoint_matches(target, got):
    if not got.get('path') or got['path'] != target['path'] or got.get('line') is None:
        return False
    if 'within' in target:
        lo, hi = target['within']
        return lo <= got['line'] <= hi
    if got.get('name') != target['name']:
        return False
    if got.get('end') is not None:
        return got['line'] <= target['line'] <= got['end']
    # Start-only output (the codegraph CLI): its start may sit on attributes or a doc comment above the name.
    return got['line'] <= target['line'] <= got['line'] + START_WINDOW


def region_matches(region, got):
    return got.get('path') == region['path'] and got.get('line') is not None \
        and region['from'] <= got['line'] <= region['to']


def score_case(case, got):
    """Edge counts for one callers/callees answer against its audited labels."""
    external = sum(1 for g in got if not g.get('path'))
    got = [g for g in got if g.get('path')]
    if case.get('site') is not None:
        got = [g for g in got if g.get('site') == case['site']
               or (g.get('site') is None and g.get('name') == case.get('call_name'))]
    entries = case.get('expect', [])
    forbid = case.get('forbid', [])
    matched = {}
    tp = fp = optional = dup = site_ok = site_wrong = forbidden = 0
    extras = []
    roles = []
    for g in got:
        hit = None
        role = None
        for i, entry in enumerate(entries):
            for target in entry.get('targets', []):
                if endpoint_matches(target, g):
                    hit, role = i, target.get('role', 'implementation')
                    break
            if hit is None and 'region' in entry and region_matches(entry['region'], g):
                hit = i
            if hit is not None:
                break
        if hit is None:
            fp += 1
            if any(endpoint_matches(f, g) for f in forbid):
                forbidden += 1
            extras.append(g)
            roles.append(None)
            continue
        roles.append(role)
        entry = entries[hit]
        if entry.get('required', True) is False:
            optional += 1
            continue
        if hit in matched:
            dup += 1
        else:
            matched[hit] = g
            tp += 1
        if g.get('site') is not None:
            if g['site'] in entry.get('sites', []):
                site_ok += 1
            else:
                site_wrong += 1
    required = [i for i, e in enumerate(entries) if e.get('required', True) is not False]
    fn = sum(1 for i in required if i not in matched)
    falsely_unique = False
    if case.get('certainty') == 'unknown':
        falsely_unique = len(got) == 1
    elif case.get('certainty') == 'indirect':
        falsely_unique = len(got) == 1 and roles[0] != 'declaration'
    return {'tp': tp, 'fp': fp, 'fn': fn, 'optional': optional, 'dup': dup, 'site_ok': site_ok,
            'site_wrong': site_wrong, 'forbidden': forbidden, 'external': external,
            'falsely_unique': falsely_unique, 'correct_empty': case.get('class') == 'miss' and not got,
            'extras': [{k: g.get(k) for k in ('path', 'line', 'end', 'name', 'site', 'certainty')} for g in extras]}


def score_refs(case, got):
    """Reference lines against labelled lines; the definition's own line is neither right nor wrong."""
    sym = case['symbol']
    lines = {(p, n) for p, n in got if (p, n) != (sym['path'], sym['line'])}
    expected = {(e['line_ref']['path'], e['line_ref']['line']): e.get('required', True) is not False
                for e in case.get('expect', [])}
    tp = sum(1 for k in lines if expected.get(k) is True)
    optional = sum(1 for k in lines if expected.get(k) is False)
    extras = sorted(k for k in lines if k not in expected)
    fn = sum(1 for k, req in expected.items() if req and k not in lines)
    return {'tp': tp, 'fp': len(extras), 'fn': fn, 'optional': optional,
            'extras': [{'path': p, 'line': n} for p, n in extras]}


# ---------------------------------------------------------------------------
# Measurement


class Context:
    def __init__(self, args):
        self.run = Path(args.run).resolve()
        self.src = Path(args.src).resolve()
        self.codecite = str(Path(args.codecite).resolve()) if getattr(args, 'codecite', None) else None
        self.repos, self.producer = load_corpora()
        self.cases = load_cases()
        self.env = dict(os.environ, XDG_DATA_HOME=str(self.run / 'data'), XDG_CACHE_HOME=str(self.run / 'cache'),
                        HOME=str(self.run / 'home'), DO_NOT_TRACK='1', CODEGRAPH_NO_UPDATE_CHECK='1',
                        CODEGRAPH_TELEMETRY='0', CODEGRAPH_NO_DOWNLOAD='1', NO_COLOR='1')
        (self.run / 'home').mkdir(parents=True, exist_ok=True)
        self.pins = {}
        for name, repo in self.repos.items():
            pin = {'schema_version': self.producer['codegraph']['schema_version'],
                   'producer_version': self.producer['codegraph']['version'],
                   'extraction_version': self.producer['codegraph']['extraction_version'],
                   'sha256': repo['codegraph_sha256']}
            path = self.run / 'pins' / f'{name}.json'
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(pin))
            self.pins[name] = path

    def has_scip(self, repo):
        return 'scip_sha256' in self.repos[repo]

    def scrub(self, text):
        if self.codecite and text == self.codecite:
            return 'codecite'
        if text == sys.executable:
            return 'python3'
        return text.replace(str(self.run), '$RUN').replace(str(self.src), '$SRC').replace(str(HERE), '$HERE')


def wrap(argv):
    return ['unshare', '-rn', *argv]


def execute(ctx, argv):
    start = time.perf_counter()
    proc = subprocess.run(wrap(argv), capture_output=True, text=True, env=ctx.env, check=False)
    return proc.returncode, proc.stdout, proc.stderr, time.perf_counter() - start


def query_name(case, tool):
    sym = case['symbol']
    if case.get('query') == 'qualified' and sym.get('qualified'):
        return sym['qualified'].replace('.', '::') if tool == 'codegraph' else sym['qualified']
    return sym['name']


def resolve_scip_id(ctx, case):
    """The SCIP ID of the definition holding the case's line, from an untimed `codecite def` by bare name."""
    sym = case['symbol']
    code, out, err, _ = execute(ctx, [ctx.codecite, 'def', f'{case["repo"]}-scip', sym['name'], '--json'])
    if code != 0:
        return None
    best = None
    for row in json.loads(out).get('results', []):
        cited = parse_citation(row.get('citation'))
        if not cited or cited[0] != sym['path'] or not cited[1] <= sym['line'] <= cited[2]:
            continue
        if 'providers' in row and 'scip' not in row['providers']:
            continue
        if best is None or cited[2] - cited[1] < best[0]:
            best = (cited[2] - cited[1], row['id'])
    return best[1] if best else None


def calls_for(ctx, case, scip_ids):
    """Every measured call for one case: (arm, argv), or (arm, None) for an arm with no command."""
    repo, op, sym = case['repo'], case['op'], case['symbol']
    checkout = ctx.src / repo
    out = []

    def codecite_argv(corpus, name):
        if op == 'refs':
            return [ctx.codecite, 'refs', corpus, name, '--json']
        return [ctx.codecite, 'graph', op, corpus, name, '--json']

    out.append(('codecite', codecite_argv(repo, query_name(case, 'codecite'))))
    if ctx.has_scip(repo):
        out.append(('codecite+scip/name', codecite_argv(f'{repo}-scip', query_name(case, 'codecite'))))
        sid = scip_ids.get(case['id'])
        out.append(('codecite+scip/id', codecite_argv(f'{repo}-scip', sid) if sid else None))
    if op == 'refs':
        out.append(('codegraph/cli', None))
    else:
        out.append(('codegraph/cli', ['codegraph', op, query_name(case, 'codegraph'), '-p', str(checkout), '--json',
                                      '--limit', str(LIMIT)]))
    out.append(('codegraph/normalized', [sys.executable, '-I', str(HERE / 'normalize.py'),
                                         str(checkout / '.codegraph' / 'codegraph.db'), str(checkout),
                                         str(ctx.pins[repo]), op, f'{sym["path"]}:{sym["line"]}']))
    return out


def def_calls_for(ctx, case):
    name = query_name(case, 'codecite')
    return [('def/codecite', [ctx.codecite, 'def', case['repo'], name, '--json']),
            ('def/codegraph', ['codegraph', 'query', query_name(case, 'codegraph'), '-p', str(ctx.src / case['repo']),
                               '--json', '--limit', '10'])]


def cmd_validate(args):
    repos, _ = load_corpora()
    errors = validate_cases(load_cases(), repos, args.src)
    for e in errors:
        print(e, file=sys.stderr)
    print(f'{len(load_cases())} cases, {len(errors)} errors')
    return 1 if errors else 0


def cmd_measure(args):
    ctx = Context(args)
    errors = validate_cases(ctx.cases, ctx.repos, ctx.src)
    if errors:
        for e in errors:
            print(e, file=sys.stderr)
        return 1
    snap_before = {r: normalize_sha(ctx, r) for r in ctx.repos}
    pin_seconds = verify_snapshots(ctx)
    scip_ids = {c['id']: resolve_scip_id(ctx, c) for c in ctx.cases if ctx.has_scip(c['repo'])}
    calls = []
    for case in ctx.cases:
        for arm, argv in calls_for(ctx, case, scip_ids) + def_calls_for(ctx, case):
            calls.append({'case': case['id'], 'repo': case['repo'], 'arm': arm, 'op': case['op'] if not
                          arm.startswith('def/') else 'def', 'argv': argv})
    out_dir = ctx.run / 'measure'
    out_dir.mkdir(parents=True, exist_ok=True)
    # Untimed sweep: the outputs that are scored.
    for i, call in enumerate(calls):
        if call['argv'] is None:
            call.update(exit=None, status='unsupported' if call['arm'] == 'codegraph/cli' else 'unresolved')
            continue
        code, out, err, elapsed = execute(ctx, call['argv'])
        (out_dir / f'{i}.out').write_text(out)
        (out_dir / f'{i}.err').write_text(err)
        call.update(exit=code, stdout_bytes=len(out.encode()), stderr_bytes=len(err.encode()),
                    stdout_sha256=hashlib.sha256(out.encode()).hexdigest(), sweep_seconds=elapsed, times=[],
                    unstable=0)
    # Timed rounds: shuffled with a fixed seed, alternating direction, one process at a time.
    order = [i for i, c in enumerate(calls) if c['argv'] is not None]
    for r in range(args.rounds):
        rng = random.Random(295 + r)
        seq = order[:]
        rng.shuffle(seq)
        if r % 2:
            seq.reverse()
        for i in seq:
            code, out, err, elapsed = execute(ctx, calls[i]['argv'])
            calls[i]['times'].append(elapsed)
            if hashlib.sha256(out.encode()).hexdigest() != calls[i]['stdout_sha256']:
                calls[i]['unstable'] += 1
    snap_after = {r: normalize_sha(ctx, r) for r in ctx.repos}
    meta = {'pin_hash_seconds': pin_seconds, 'rounds': args.rounds, 'seed': 295, 'limit': LIMIT, 'loadavg_end': os.getloadavg(),
            'snapshots_unchanged': snap_before == snap_after, 'scip_ids_resolved': sum(1 for v in scip_ids.values() if v),
            'scip_ids_wanted': len(scip_ids)}
    (ctx.run / 'calls.json').write_text(json.dumps({'meta': meta, 'calls': calls, 'scip_ids': scip_ids}, indent=1))
    print(json.dumps(meta))
    return 0


def verify_snapshots(ctx):
    """Hash each private CodeGraph snapshot once against its pin and stamp it, so queries need not rehash it."""
    sys.path.insert(0, str(HERE))
    import normalize
    seconds = {}
    for name in ctx.repos:
        db = ctx.src / name / '.codegraph' / 'codegraph.db'
        pin = json.loads(ctx.pins[name].read_text())
        start = time.perf_counter()
        actual = normalize.sha256_file(db)
        seconds[name] = round(time.perf_counter() - start, 2)
        if actual != pin['sha256']:
            raise SystemExit(f'{name}: snapshot sha256 {actual} does not match corpora.toml')
        pin['verified_stamp'] = normalize.stamp(db)
        ctx.pins[name].write_text(json.dumps(pin))
    return seconds


def normalize_sha(ctx, repo):
    """The snapshot's sha256, read in blocks: Linux's snapshot is 4.7 GB."""
    sys.path.insert(0, str(HERE))
    import normalize
    db = ctx.src / repo / '.codegraph' / 'codegraph.db'
    return normalize.sha256_file(db) if db.exists() else None


# ---------------------------------------------------------------------------
# Untimed checks: citation round trips and artifact file hashes


def cmd_check(args):
    ctx = Context(args)
    data = json.loads((ctx.run / 'calls.json').read_text())
    citations = defaultdict(set)
    out_dir = ctx.run / 'measure'
    for i, call in enumerate(data['calls']):
        if call['argv'] is None or call['exit'] != 0 or not call['arm'].startswith(('codecite', 'def/codecite')):
            continue
        corpus = call['argv'][3] if call['arm'] != 'def/codecite' else call['argv'][2]
        if call['op'] == 'refs':
            corpus = call['argv'][2]
        try:
            rows = json.loads((out_dir / f'{i}.out').read_text()).get('results', [])
        except ValueError:
            continue
        for row in rows:
            for key in ('citation', 'call_site'):
                if row.get(key):
                    citations[corpus].add(row[key])
    readback = {'checked': 0, 'failed': []}
    for corpus, cites in sorted(citations.items()):
        for cite in sorted(cites):
            code, _, err, _ = execute(ctx, [ctx.codecite, 'read', corpus, cite])
            readback['checked'] += 1
            if code != 0:
                readback['failed'].append({'corpus': corpus, 'citation': cite, 'error': err.strip()[:200]})
    # CodeGraph CLI endpoints: the path must exist in the checkout and the start line must be in the file.
    cli = {'checked': 0, 'failed': []}
    for i, call in enumerate(data['calls']):
        if call['arm'] != 'codegraph/cli' or call['argv'] is None or call['exit'] != 0:
            continue
        try:
            got = parse_codegraph((out_dir / f'{i}.out').read_text(), call['op'])
        except ValueError:
            continue
        for g in got:
            cli['checked'] += 1
            path = ctx.src / call['repo'] / (g['path'] or '')
            try:
                n = len(path.read_text(errors='replace').splitlines())
            except OSError:
                n = 0
            if not g['path'] or not 1 <= (g['line'] or 0) <= n:
                cli['failed'].append({'case': call['case'], 'path': g['path'], 'line': g['line']})
    # Artifact file table against the checkout: every file CodeGraph recorded, by sha256.
    sys.path.insert(0, str(HERE))
    import normalize
    artifacts = {}
    for name in ctx.repos:
        art = normalize.Artifact(ctx.src / name / '.codegraph' / 'codegraph.db', ctx.src / name,
                                 json.loads(ctx.pins[name].read_text()))
        rows = art.con.execute('SELECT path, content_hash FROM files ORDER BY path').fetchall()
        digest = hashlib.sha256()
        problems = Counter()
        examples = []
        for row in rows:
            digest.update(f'{row["path"]}\0{row["content_hash"]}\n'.encode())
            found = art.check_file(row['path'])
            for p in found:
                kind = p.split(': ', 1)[1] if ': ' in p else p
                problems[kind] += 1
                if len(examples) < 5:
                    examples.append(p)
        tracked = set(git(ctx.src / name, 'ls-files').splitlines())
        recorded = {r['path'] for r in rows}
        artifacts[name] = {'files': len(rows), 'verified': len(rows) - sum(problems.values()),
                           'problems': dict(problems), 'examples': examples,
                           'file_table_sha256': digest.hexdigest(),
                           'tracked_files_not_recorded': len(tracked - recorded),
                           'nodes': art.con.execute('SELECT count(*) FROM nodes').fetchone()[0],
                           'edges': dict(art.con.execute('SELECT kind, count(*) FROM edges GROUP BY kind').fetchall()),
                           'unresolved_refs': art.con.execute('SELECT count(*) FROM unresolved_refs').fetchone()[0],
                           'calls_without_confidence': art.con.execute(
                               "SELECT count(*) FROM edges WHERE kind = 'calls' AND "
                               "(metadata IS NULL OR json_extract(metadata, '$.confidence') IS NULL)").fetchone()[0],
                           'calls_below_0_7': art.con.execute(
                               "SELECT count(*) FROM edges WHERE kind = 'calls' AND "
                               "json_extract(metadata, '$.confidence') < 0.7").fetchone()[0],
                           'edges_without_line': art.con.execute(
                               'SELECT count(*) FROM edges WHERE line IS NULL AND kind != \'contains\'').fetchone()[0]}
        art.close()
    (ctx.run / 'check.json').write_text(json.dumps({'readback': readback, 'codegraph_cli_paths': cli,
                                                    'artifacts': artifacts}, indent=1))
    print(json.dumps({'readback_checked': readback['checked'], 'readback_failed': len(readback['failed']),
                      'cli_checked': cli['checked'], 'cli_failed': len(cli['failed'])}))
    return 0


# ---------------------------------------------------------------------------
# Report


def pct(values, q):
    if not values:
        return None
    values = sorted(values)
    k = (len(values) - 1) * q
    lo, hi = int(k), min(int(k) + 1, len(values) - 1)
    return values[lo] + (values[hi] - values[lo]) * (k - lo)


def load_audit():
    path = HERE / 'audit.json'
    if not path.exists():
        return {}
    audit = {}
    for row in json.loads(path.read_text())['extras']:
        audit[(row['case'], row['path'], row['line'])] = row
    return audit


def outcome_rows(ctx, data):
    """One row per case and arm: status, cited answer and score."""
    cases = {c['id']: c for c in ctx.cases}
    out_dir = ctx.run / 'measure'
    rows = []
    by_case = defaultdict(dict)
    for i, call in enumerate(data['calls']):
        by_case[call['case']][call['arm']] = (i, call)
    for case in ctx.cases:
        arms = by_case[case['id']]
        for arm in ARMS:
            source_arm = 'codegraph/normalized' if arm == 'codegraph/normalized-trusted' else arm
            row = {'case': case['id'], 'repo': case['repo'], 'split': ctx.repos[case['repo']]['split'],
                   'class': case['class'], 'op': case['op'], 'certainty': case['certainty'], 'arm': arm}
            if source_arm not in arms:
                row['status'] = 'unavailable'
                rows.append(row)
                continue
            i, call = arms[source_arm]
            if call['argv'] is None:
                row['status'] = call['status']
                rows.append(row)
                continue
            out = (out_dir / f'{i}.out').read_text()
            err = (out_dir / f'{i}.err').read_text()
            unresolved = []
            if arm.startswith('codecite'):
                status = codecite_status(call['exit'], out, err)
                got = parse_codecite(out, case['op']) if status == 'ok' else []
            elif arm == 'codegraph/cli':
                status = codegraph_status(call['exit'], out, err)
                got = parse_codegraph(out, case['op']) if status == 'ok' else []
            else:
                try:
                    parsed = json.loads(out)
                except ValueError:
                    parsed = {'status': 'error'}
                status = parsed.get('status', 'error')
                status = 'ok' if status == 'ok' else ('rejected' if status == 'rejected' else status)
                got, unresolved = parse_normalized(out, case['op'], trusted=arm.endswith('trusted')) \
                    if status == 'ok' else ([], [])
            row['status'] = status
            row['truncated'] = is_truncated(arm, status, got)
            if status == 'ok' and arm.startswith('codecite') and case['op'] != 'refs':
                total = json.loads(out).get('total', len(got))
                row['truncated'] = total > len(got)
            if case['op'] == 'refs':
                row['answer'] = [{'path': g['path'], 'line': g['line']} for g in got]
                row['score'] = score_refs(case, [(g['path'], g['line']) for g in got])
            else:
                row['answer'] = [{k: g.get(k) for k in ('path', 'line', 'end', 'name', 'site', 'certainty', 'evidence')
                                  if g.get(k) is not None} for g in got]
                row['score'] = score_case(case, got)
                if case.get('site') is not None and unresolved:
                    row['reported_unknown'] = any(u['site']['line'] == case['site'] for u in unresolved)
            if case['certainty'] == 'ambiguous' and arm.endswith(('/name', '/cli')) or (
                    case['certainty'] == 'ambiguous' and arm == 'codecite'):
                row['ambiguity'] = 'exposed' if status == 'ambiguous' else ('hidden' if status == 'ok' else status)
            row['seconds'] = call['times']
            rows.append(row)
    return rows


def is_truncated(arm, status, got):
    """Only `codegraph/cli` is asked with `--limit`; codecite reports its own total, the normalized arms have no limit."""
    return arm == 'codegraph/cli' and status == 'ok' and len(got) >= LIMIT


def apply_audit(rows, audit):
    """Audited extras. A `label-miss` (a real edge the labels lacked) counts as a required label of its case: one
    tp for each arm that returned it, however often, and one fn for each arm that did not. `false` stays fp."""
    missed_labels = defaultdict(set)
    for (case, path, line), verdict in audit.items():
        if verdict['verdict'] == 'label-miss':
            missed_labels[case].add((path, line))
    for row in rows:
        score = row.get('score')
        if not score:
            continue
        audited = dict(score)
        found = set()
        repeats = 0
        unaudited = 0
        for extra in score.get('extras', []):
            key = (extra['path'], extra['line'])
            verdict = audit.get((row['case'], *key))
            if verdict is None:
                unaudited += 1
            elif verdict['verdict'] == 'label-miss':
                if key in found:
                    repeats += 1
                found.add(key)
        label_miss = len(found)
        audited['tp'] = score['tp'] + label_miss
        audited['fp'] = score['fp'] - label_miss - repeats
        audited['fn'] = score['fn'] + len(missed_labels[row['case']] - found)
        if repeats:
            audited['dup'] = score.get('dup', 0) + repeats
        audited['label_miss'] = label_miss
        audited['unaudited'] = unaudited
        row['audited'] = audited


def aggregate(rows, key='audited'):
    agg = defaultdict(lambda: Counter())
    for row in rows:
        a = agg[row['arm']]
        a['cases'] += 1
        a[f'status:{row["status"]}'] += 1
        s = row.get(key)
        if not s:
            continue
        a['tp'] += s['tp']
        a['fp'] += s['fp']
        a['fn'] += s['fn']
        for k in ('site_ok', 'site_wrong', 'forbidden', 'label_miss', 'unaudited', 'external', 'dup', 'optional'):
            a[k] += s.get(k, 0)
        if row['certainty'] in ('unknown', 'indirect'):
            a['indirect_or_unknown_cases'] += 1
            a['falsely_unique'] += int(s.get('falsely_unique', False))
        if row['class'] == 'miss':
            a['miss_cases'] += 1
            a['correct_empty'] += int(s.get('correct_empty', False))
        if row.get('truncated'):
            a['truncated'] += 1
        if row.get('reported_unknown'):
            a['reported_unknown'] += 1
    for a in agg.values():
        a['precision'] = round(a['tp'] / (a['tp'] + a['fp']), 3) if a['tp'] + a['fp'] else None
        a['recall'] = round(a['tp'] / (a['tp'] + a['fn']), 3) if a['tp'] + a['fn'] else None
    return {k: dict(v) for k, v in agg.items()}


def cmd_report(args):
    ctx = Context(args)
    data = json.loads((ctx.run / 'calls.json').read_text())
    check = json.loads((ctx.run / 'check.json').read_text()) if (ctx.run / 'check.json').exists() else {}
    rows = outcome_rows(ctx, data)
    apply_audit(rows, load_audit())
    results = HERE / 'results'
    results.mkdir(exist_ok=True)
    with open(results / 'per-case.jsonl', 'w') as fh:
        for row in rows:
            fh.write(json.dumps({k: v for k, v in row.items() if k != 'seconds'}) + '\n')
    with open(results / 'calls.jsonl', 'w') as fh:
        for call in data['calls']:
            rec = {k: call.get(k) for k in ('case', 'repo', 'arm', 'op', 'exit', 'status', 'stdout_bytes',
                                           'stderr_bytes', 'sweep_seconds', 'times', 'unstable')}
            rec['argv'] = [ctx.scrub(a) for a in call['argv']] if call['argv'] else None
            fh.write(json.dumps(rec) + '\n')
    splits = {'evaluation': [r for r in rows if r['split'] == 'evaluation'],
              'control': [r for r in rows if r['split'] == 'control']}
    summary = {'by_split': {s: aggregate(rs) for s, rs in splits.items()},
               'by_split_raw': {s: aggregate(rs, 'score') for s, rs in splits.items()},
               'by_repo': {r: aggregate([x for x in rows if x['repo'] == r]) for r in ctx.repos},
               'by_class': {c: aggregate([x for x in splits['evaluation'] if x['class'] == c])
                            for c in sorted(CLASSES)},
               'by_op': {o: aggregate([x for x in splits['evaluation'] if x['op'] == o]) for o in sorted(OPS)},
               'ambiguity': ambiguity(rows),
               'latency': latency(data), 'index': index_costs(ctx), 'check': summarize_check(check),
               'meta': data['meta']}
    (results / 'summary.json').write_text(json.dumps(summary, indent=1, default=str))
    (results / 'summary.md').write_text(render_md(summary))
    (results / 'metadata.json').write_text(json.dumps(metadata(ctx, data), indent=1))
    print((results / 'summary.md').read_text())
    return 0


def ambiguity(rows):
    out = defaultdict(Counter)
    for row in rows:
        if 'ambiguity' in row:
            out[row['arm']][row['ambiguity']] += 1
    return {k: dict(v) for k, v in out.items()}


def latency(data):
    out = defaultdict(lambda: defaultdict(list))
    for call in data['calls']:
        if call.get('times'):
            out[call['repo']][call['arm']].extend(call['times'])
    return {repo: {arm: {'n': len(t), 'p50_ms': round(1000 * pct(t, 0.5), 1), 'p95_ms': round(1000 * pct(t, 0.95), 1)}
                   for arm, t in sorted(arms.items())} for repo, arms in sorted(out.items())}


def time_line(path):
    """Wall seconds and peak RSS from GNU `time -f '%e %M' -o`, which puts a "Command exited with non-zero status"
    line first when the timed command fails."""
    return path.read_text().strip().splitlines()[-1].split()


def index_costs(ctx):
    out = {}
    for name, repo in ctx.repos.items():
        row = {}
        for corpus in (name, f'{name}-scip'):
            t = ctx.run / 'index' / f'{corpus}.codecite.time'
            db = ctx.run / 'data' / 'codecite' / f'{corpus}.db'
            if t.exists():
                wall, rss = time_line(t)
                row[corpus] = {'wall_seconds': float(wall), 'peak_rss_kib': int(rss),
                               'db_bytes': db.stat().st_size if db.exists() else None}
        cg = ctx.src / name / '.codegraph' / 'codegraph.db'
        row['codegraph_db_bytes'] = cg.stat().st_size if cg.exists() else None
        scip = ctx.run.parent / 'scip' / f'{name}.scip'
        row['scip_bytes'] = scip.stat().st_size if scip.exists() else None
        st = ctx.run.parent / 'logs' / f'{name}.scip.time'
        if st.exists():
            wall, rss = time_line(st)
            row['scip_indexer'] = {'wall_seconds': float(wall), 'peak_rss_kib': int(rss)}
        out[name] = row
    for name in out:
        if 'codegraph_init' in ctx.repos[name]:
            out[name]['codegraph_init'] = ctx.repos[name]['codegraph_init']
    return out


def summarize_check(check):
    if not check:
        return {}
    return {'codecite_readback': {'checked': check['readback']['checked'], 'failed': len(check['readback']['failed']),
                                  'failures': check['readback']['failed'][:10]},
            'codegraph_cli_paths': {'checked': check['codegraph_cli_paths']['checked'],
                                    'failed': len(check['codegraph_cli_paths']['failed'])},
            'artifacts': check['artifacts']}


def fmt(v):
    return '–' if v is None else (f'{v:.2f}' if isinstance(v, float) else str(v))


def render_md(summary):
    lines = ['# Results', '', 'Generated by `run.py report`. Precision and recall count audited edges '
             '(`audit.json`); `by_split_raw` in summary.json has the unaudited counts.', '']
    for split, agg in summary['by_split'].items():
        lines += [f'## {split.capitalize()} repositories', '',
                  '| Arm | Cases | ok | unsupported | unavailable | ambiguous | Edge P | Edge R | TP | FP | FN | '
                  'Site ok/wrong | Forbidden hits | Falsely unique | Correct empty |',
                  '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | ---: | --- | --- |']
        for arm in ARMS:
            a = agg.get(arm)
            if not a:
                continue
            lines.append(f'| {arm} | {a["cases"]} | {a.get("status:ok", 0)} | {a.get("status:unsupported", 0)} | '
                         f'{a.get("status:unavailable", 0)} | {a.get("status:ambiguous", 0)} | {fmt(a["precision"])} | '
                         f'{fmt(a["recall"])} | {a.get("tp", 0)} | {a.get("fp", 0)} | {a.get("fn", 0)} | '
                         f'{a.get("site_ok", 0)}/{a.get("site_wrong", 0)} | {a.get("forbidden", 0)} | '
                         f'{a.get("falsely_unique", 0)}/{a.get("indirect_or_unknown_cases", 0)} | '
                         f'{a.get("correct_empty", 0)}/{a.get("miss_cases", 0)} |')
        lines.append('')
    lines += ['## Evaluation repositories by operation', '', '| Op | Arm | Edge P | Edge R | TP | FP | FN |',
              '| --- | --- | ---: | ---: | ---: | ---: | ---: |']
    for op, agg in summary['by_op'].items():
        for arm in ARMS:
            a = agg.get(arm)
            if a and (a.get('tp') or a.get('fp') or a.get('fn')):
                lines.append(f'| {op} | {arm} | {fmt(a["precision"])} | {fmt(a["recall"])} | {a.get("tp", 0)} | '
                             f'{a.get("fp", 0)} | {a.get("fn", 0)} |')
    lines += ['', '## Evaluation repositories by class', '', '| Class | Arm | Edge P | Edge R | TP | FP | FN |',
              '| --- | --- | ---: | ---: | ---: | ---: | ---: |']
    for cls, agg in summary['by_class'].items():
        for arm in ARMS:
            a = agg.get(arm)
            if a and (a.get('tp') or a.get('fp') or a.get('fn')):
                lines.append(f'| {cls} | {arm} | {fmt(a["precision"])} | {fmt(a["recall"])} | {a.get("tp", 0)} | '
                             f'{a.get("fp", 0)} | {a.get("fn", 0)} |')
    lines += ['', '## Per repository', '', '| Repository | Arm | Edge P | Edge R | TP | FP | FN | Status |',
              '| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |']
    for repo, agg in summary['by_repo'].items():
        for arm in ARMS:
            a = agg.get(arm)
            if not a:
                continue
            st = ', '.join(f'{k[7:]} {v}' for k, v in sorted(a.items()) if k.startswith('status:'))
            lines.append(f'| {repo} | {arm} | {fmt(a["precision"])} | {fmt(a["recall"])} | {a.get("tp", 0)} | '
                         f'{a.get("fp", 0)} | {a.get("fn", 0)} | {st} |')
    lines += ['', '## Bare-name ambiguity (cases whose name has several definitions)', '',
              '| Arm | Exposed | Hidden (answered for one or merged) | Other |', '| --- | ---: | ---: | --- |']
    for arm, c in summary['ambiguity'].items():
        other = ', '.join(f'{k} {v}' for k, v in c.items() if k not in ('exposed', 'hidden'))
        lines.append(f'| {arm} | {c.get("exposed", 0)} | {c.get("hidden", 0)} | {other} |')
    lines += ['', '## Latency (startup included, timed rounds)', '', '| Repository | Arm | n | p50 ms | p95 ms |',
              '| --- | --- | ---: | ---: | ---: |']
    for repo, arms in summary['latency'].items():
        for arm, t in arms.items():
            lines.append(f'| {repo} | {arm} | {t["n"]} | {t["p50_ms"]} | {t["p95_ms"]} |')
    lines += ['', '## Index cost', '', '| Repository | codecite s / MiB RSS / MB | codecite+SCIP s / MiB RSS / MB | '
              'SCIP indexer s / MiB RSS / file MB | CodeGraph init s / MiB RSS / MB |', '| --- | --- | --- | --- | --- |']
    for repo, row in summary['index'].items():
        def cell(d, size_key='db_bytes'):
            if not d:
                return 'unavailable'
            size = d.get(size_key)
            return (f'{d["wall_seconds"]:.1f} / {d["peak_rss_kib"] // 1024} / '
                    f'{(size or 0) / 1e6:.0f}')
        cg = row.get('codegraph_init')
        cg_cell = (f'{cg["wall_seconds"]:.1f} / {cg["peak_rss_kib"] // 1024} / {cg["db_bytes"] / 1e6:.0f}'
                   if cg else 'unrecorded')
        scip = row.get('scip_indexer')
        scip_cell = (f'{scip["wall_seconds"]:.1f} / {scip["peak_rss_kib"] // 1024} / '
                     f'{"–" if row["scip_bytes"] is None else format(row["scip_bytes"] / 1e6, ".0f")}'
                     if scip else 'unavailable')
        lines.append(f'| {repo} | {cell(row.get(repo))} | {cell(row.get(repo + "-scip"))} | {scip_cell} | {cg_cell} |')
    check = summary.get('check') or {}
    if check:
        rb = check['codecite_readback']
        lines += ['', '## Source and citation checks', '',
                  f'- codecite citations read back: {rb["checked"] - rb["failed"]} of {rb["checked"]}.',
                  f'- codegraph CLI endpoints whose path and start line exist in the checkout: '
                  f'{check["codegraph_cli_paths"]["checked"] - check["codegraph_cli_paths"]["failed"]} of '
                  f'{check["codegraph_cli_paths"]["checked"]} (the CLI prints no hash or end line to check further).',
                  '', '| Repository | CodeGraph files | Verified against checkout | Problems | Calls < 0.7 confidence | '
                  'Non-containment edges without a line | Unresolved refs |', '| --- | ---: | ---: | --- | ---: | ---: | ---: |']
        for repo, a in check['artifacts'].items():
            probs = ', '.join(f'{k} {v}' for k, v in a['problems'].items()) or 'none'
            lines.append(f'| {repo} | {a["files"]} | {a["verified"]} | {probs} | {a["calls_below_0_7"]} of '
                         f'{a["edges"].get("calls", 0)} | {a["edges_without_line"]} | {a["unresolved_refs"]} |')
    lines.append('')
    return '\n'.join(lines)


def metadata(ctx, data):
    def version(argv):
        try:
            return subprocess.run(argv, capture_output=True, text=True, check=False, env=ctx.env).stdout.strip()
        except OSError:
            return None
    return {'platform': platform.platform(), 'cpus': os.cpu_count(), 'python': platform.python_version(),
            'codecite_version': version([ctx.codecite, '--version']) if ctx.codecite else None,
            'codegraph_version': version(['codegraph', '--version']),
            'cases_sha256': hashlib.sha256((HERE / 'cases.json').read_bytes()).hexdigest(),
            'corpora_sha256': hashlib.sha256((HERE / 'corpora.toml').read_bytes()).hexdigest(),
            'normalize_sha256': hashlib.sha256((HERE / 'normalize.py').read_bytes()).hexdigest(),
            'pins': [{'repo': n, 'commit': r['commit']} for n, r in ctx.repos.items()], 'measure': data['meta']}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='cmd', required=True)
    for name in ('validate', 'measure', 'check', 'report'):
        p = sub.add_parser(name)
        p.add_argument('--src', required=True)
        if name != 'validate':
            p.add_argument('--run', required=True)
        if name in ('measure', 'check', 'report'):
            p.add_argument('--codecite', required=name != 'report')
        if name == 'measure':
            p.add_argument('--rounds', type=int, default=5)
    args = parser.parse_args()
    if shutil.which('unshare') is None and args.cmd in ('measure', 'check'):
        print('unshare is required', file=sys.stderr)
        return 2
    return {'validate': cmd_validate, 'measure': cmd_measure, 'check': cmd_check, 'report': cmd_report}[args.cmd](args)


if __name__ == '__main__':
    sys.exit(main())
