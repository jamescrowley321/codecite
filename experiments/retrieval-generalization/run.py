#!/usr/bin/env python3
"""Direct-CLI generic retrieval benchmark for #287. See README.md.

    run.py validate --src DIR
    run.py measure  --run RUN --src DIR --codecite BIN [--rounds 5] [--seed 287]
    run.py diagnose --run RUN --src DIR --codecite BIN --helper BIN
    run.py report   --run RUN --out results

Every measured call is a fresh local process of `codecite` or `codegraph`
under `unshare -rn` (no network), with codecite's data/cache homes and
CodeGraph's HOME inside RUN. No model, agent or MCP server is involved.
Raw stdout/stderr stay in RUN; `report` writes only citations (path and
lines), ranks, sizes, times and classifications to the committed results.
"""

import argparse
import hashlib
import json
import os
import platform
import random
import re
import sqlite3
import subprocess
import sys
import time
import tomllib
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
LIMIT = 10
SPLITS = ('development', 'evaluation')
CLASSES = ('exact', 'qualified', 'ambiguous', 'behaviour', 'path', 'term', 'broad', 'no-result')
DEFINITION_CLASSES = ('exact', 'qualified')
POSITIVE = ('exact', 'qualified', 'behaviour', 'path', 'term')
# The operations each class asks for; each codecite op is paired with the
# CodeGraph op that answers the same question, for interleaved timing.
OPS = {
    'exact': ['codecite:def', 'codecite:search', 'codegraph:node', 'codegraph:query'],
    'qualified': ['codecite:def', 'codecite:search', 'codegraph:node', 'codegraph:query'],
    'ambiguous': ['codecite:def', 'codecite:search', 'codegraph:node', 'codegraph:query'],
    'behaviour': ['codecite:search', 'codegraph:query'],
    'path': ['codecite:search', 'codegraph:query'],
    'term': ['codecite:search', 'codegraph:query'],
    'broad': ['codecite:search', 'codegraph:query'],
    'no-result': ['codecite:def', 'codecite:search', 'codegraph:node', 'codegraph:query'],
}
PARTNER = {'def': 'node', 'search': 'query'}
NOT_FOUND = re.compile(r'defines no symbol')


# ---------------------------------------------------------------- inputs

def load_corpora(path=HERE / 'corpora.toml'):
    return {r['name']: r for r in tomllib.loads(Path(path).read_text())['repo']}


def load_cases(path=HERE / 'cases.json'):
    return json.loads(Path(path).read_text())['cases']


def ops_of(case):
    return case.get('ops') or OPS[case['class']]


def last_name(query):
    return re.split(r'[.:#]+', query.strip())[-1]


def validate(corpora, cases, head_of, read_line):
    """Every problem with the workload, before any CLI is spawned."""
    errors = []
    urls = defaultdict(set)
    for name, repo in corpora.items():
        if repo.get('split') not in SPLITS:
            errors.append(f'{name}: split must be one of {SPLITS}')
        urls[repo.get('url', name).rstrip('/').removesuffix('.git').lower()].add(repo.get('split'))
    for url, splits in urls.items():
        if len(splits) > 1:
            errors.append(f'{url}: repository is in both splits')
    seen = set()
    heads = {}
    for case in cases:
        cid = case.get('id', '?')
        if cid in seen:
            errors.append(f'{cid}: duplicate id')
        seen.add(cid)
        repo = corpora.get(case.get('repo'))
        if repo is None:
            errors.append(f'{cid}: unknown repository {case.get("repo")}')
            continue
        if case.get('class') not in CLASSES:
            errors.append(f'{cid}: unknown class {case.get("class")}')
            continue
        if case.get('revision') != repo['commit']:
            errors.append(f'{cid}: revision is not the pin {repo["commit"]}')
        if repo['name'] not in heads:
            heads[repo['name']] = head_of(repo['name'])
        if heads[repo['name']] != repo['commit']:
            errors.append(f'{cid}: checkout of {repo["name"]} is not at the pin')
        answers = case.get('answers') or []
        if case['class'] == 'no-result' and answers:
            errors.append(f'{cid}: a no-result case has answers')
        if case['class'] in POSITIVE and case.get('score', True) and not answers:
            errors.append(f'{cid}: a scored {case["class"]} case needs answers')
        if case['class'] == 'path' and not case.get('path'):
            errors.append(f'{cid}: a path case needs a path filter')
        for op in case.get('ops') or []:
            if op not in OPS['exact']:
                errors.append(f'{cid}: unknown op {op}')
        for answer in answers:
            text = read_line(repo['name'], answer['path'], answer['line'])
            if text is None:
                errors.append(f'{cid}: no line {answer["path"]}:{answer["line"]} at the pin')
            elif case['class'] in DEFINITION_CLASSES and last_name(case['query']) not in text:
                errors.append(f'{cid}: {answer["path"]}:{answer["line"]} does not name {last_name(case["query"])}')
    return errors


def check_or_exit(corpora, cases, head_of, read_line):
    errors = validate(corpora, cases, head_of, read_line)
    if errors:
        sys.exit('invalid workload; nothing was run:\n  ' + '\n  '.join(errors))


def git_head(src):
    def head_of(repo):
        try:
            return subprocess.check_output(['git', '-C', str(Path(src) / repo), 'rev-parse', 'HEAD'],
                                           text=True, stderr=subprocess.DEVNULL).strip()
        except (subprocess.CalledProcessError, FileNotFoundError):
            return None
    return head_of


def checkout_lines(src):
    cache = {}

    def read_line(repo, path, line):
        key = (repo, path)
        if key not in cache:
            try:
                cache[key] = (Path(src) / repo / path).read_text(errors='replace').splitlines()
            except OSError:
                cache[key] = None
        lines = cache[key]
        return lines[line - 1] if lines and 1 <= line <= len(lines) else None
    return read_line


# ---------------------------------------------------------------- calls

def argv(tool, op, case, codecite, src):
    """The command for one call, or None when the tool lacks the operation."""
    q = case['query']
    if tool == 'codecite':
        if op == 'def':
            return [codecite, 'def', case['repo'], q, '--json']
        cmd = [codecite, 'search', case['repo'], q, '--limit', str(LIMIT), '--json']
        return cmd + (['--path', case['path']] if case.get('path') else [])
    checkout = str(Path(src) / case['repo'])
    if case.get('path'):
        return None  # CodeGraph's query has no path filter.
    if op == 'node':
        return ['codegraph', 'node', q, '-p', checkout]
    return ['codegraph', 'query', q, '-p', checkout, '--limit', str(LIMIT), '--json']


def environment(run):
    return dict(os.environ, XDG_DATA_HOME=str(run / 'data'), XDG_CACHE_HOME=str(run / 'cache'),
                HOME=str(run / 'home'), DO_NOT_TRACK='1', CODEGRAPH_NO_UPDATE_CHECK='1',
                CODEGRAPH_TELEMETRY='0', NO_COLOR='1')


def call(cmd, env):
    start = time.perf_counter_ns()
    proc = subprocess.run(['unshare', '-rn', *cmd], env=env, capture_output=True)
    elapsed = (time.perf_counter_ns() - start) / 1e6
    return proc, elapsed


def parse_citation(citation):
    _, _, rest = citation.partition('@')
    _, _, rest = rest.partition(':')
    path, _, lines = rest.rpartition('#L')
    start, _, end = lines.partition('-L')
    return path, int(start), int(end or start)


def parse(tool, op, exit_code, stdout, stderr):
    """(status, [(path, start, end)], total) for one call's output."""
    try:
        if tool == 'codecite':
            if exit_code != 0:
                if op == 'def' and NOT_FOUND.search(stderr):
                    return 'empty', [], 0
                return 'error', [], 0
            data = json.loads(stdout)
            rows = [parse_citation(r['citation']) for r in data['results']]
            total = data.get('total', len(rows)) if op == 'def' else len(rows)
            return ('ok' if rows else 'empty'), rows, total
        if exit_code != 0:
            return 'error', [], 0
        if op == 'query':
            rows = [(r['node']['filePath'], r['node']['startLine'], r['node']['endLine'])
                    for r in json.loads(stdout)]
            return ('ok' if rows else 'empty'), rows, len(rows)
        if re.search(r'^Symbol ".*" not found', stdout, re.M):
            return 'empty', [], 0
        rows = []
        for block in re.split(r'(?=\*\*Location:\*\* )', stdout)[1:]:
            m = re.match(r'\*\*Location:\*\* (.+):(\d+)\s*$', block.splitlines()[0])
            if not m:
                continue
            start = int(m.group(2))
            numbered = [int(n) for n in re.findall(r'^(\d+)\t', block, re.M)]
            rows.append((m.group(1), start, max([start, *numbered])))
        return ('ok' if rows else 'error'), rows, len(rows)
    except (ValueError, KeyError, TypeError):
        return 'error', [], 0


def first_hit(rows, answers):
    for rank, (path, start, end) in enumerate(rows, 1):
        for a in answers:
            if path == a['path'] and start <= a['line'] <= end:
                return rank
    return None


def schedule(pairs, rounds, seed):
    """Each round runs every call once; paired calls run back to back, the
    codecite call first in even rounds and second in odd ones (A/B, B/A)."""
    out = []
    for i in range(rounds):
        order = list(pairs)
        random.Random(f'{seed}:{i}').shuffle(order)
        calls = []
        for a, b in order:
            if b is None:
                calls.append(a)
            else:
                calls.extend([a, b] if i % 2 == 0 else [b, a])
        out.append(calls)
    return out


def digest(path):
    with open(path, 'rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def measure(args):
    run, src = Path(args.run).resolve(), Path(args.src).resolve()
    corpora, cases = load_corpora(), load_cases()
    check_or_exit(corpora, cases, git_head(src), checkout_lines(src))
    out = run / 'measure'
    out.mkdir(exist_ok=False)
    (out / 'raw').mkdir()
    env = environment(run)
    keys, pairs = {}, []
    for case in cases:
        ops = ops_of(case)
        for spec in ops:
            tool, op = spec.split(':')
            cmd = argv(tool, op, case, args.codecite, src)
            keys[(case['id'], tool, op)] = cmd
        for spec in ops:
            tool, op = spec.split(':')
            if tool != 'codecite':
                continue
            a = (case['id'], 'codecite', op)
            b = (case['id'], 'codegraph', PARTNER[op])
            b = b if f'codegraph:{PARTNER[op]}' in ops and keys.get(b) else None
            pairs.append((a, b))
        for spec in ops:
            tool, op = spec.split(':')
            k = (case['id'], tool, op)
            if tool == 'codegraph' and keys[k] and not any(b == k for _, b in pairs):
                pairs.append((k, None))
    meta = dict(started=time.strftime('%Y-%m-%dT%H:%M:%S%z'), platform=platform.platform(),
                cpus=os.cpu_count(), loadavg_start=os.getloadavg(), rounds=args.rounds, seed=args.seed, limit=LIMIT,
                codecite_version=subprocess.check_output([args.codecite, '--version'], text=True).strip(),
                codecite_sha256=digest(args.codecite),
                codegraph_version=subprocess.check_output(['codegraph', '--version'], text=True, env=env).strip(),
                cases_sha256=digest(HERE / 'cases.json'), corpora_sha256=digest(HERE / 'corpora.toml'),
                pins=[dict(repo=n, commit=r['commit']) for n, r in corpora.items()],
                warmup='one untimed sweep of every call before the timed rounds',
                network='every call under unshare -rn')
    log = (out / 'calls.jsonl').open('w')

    def one(key, sweep):
        cmd = keys[key]
        record = dict(case=key[0], tool=key[1], op=key[2], sweep=sweep)
        if cmd is None:
            record.update(status='unsupported')
        else:
            proc, elapsed = call(cmd, env)
            stem = f'{sweep}-{key[0]}-{key[1]}-{key[2]}'
            (out / 'raw' / f'{stem}.out').write_bytes(proc.stdout)
            (out / 'raw' / f'{stem}.err').write_bytes(proc.stderr)
            status, rows, total = parse(key[1], key[2], proc.returncode,
                                        proc.stdout.decode(errors='replace'), proc.stderr.decode(errors='replace'))
            record.update(argv=cmd, exit=proc.returncode, stdout_bytes=len(proc.stdout),
                          stderr_bytes=len(proc.stderr), elapsed_ms=elapsed, status=status,
                          rows=rows[:50], total=total)
        log.write(json.dumps(record) + '\n')
        log.flush()

    flat = [k for pair in pairs for k in pair if k]
    print(f'untimed sweep: {len(flat)} calls', flush=True)
    for key in flat:
        one(key, 'warmup')
    for i, calls in enumerate(schedule(pairs, args.rounds, args.seed), 1):
        print(f'round {i}: {len(calls)} calls', flush=True)
        for key in calls:
            one(key, f'round{i}')
    meta.update(finished=time.strftime('%Y-%m-%dT%H:%M:%S%z'), loadavg_end=os.getloadavg())
    (out / 'metadata.json').write_text(json.dumps(meta, indent=1) + '\n')


# ---------------------------------------------------------------- diagnosis

def parse_candidates(text):
    d = dict(top=[], targets={}, docs={})
    for line in text.splitlines():
        f = line.split('\t')
        if f[0] == 'total':
            d['total'] = int(f[1])
        elif f[0] == 'deftotal':
            d['deftotal'] = int(f[1])
        elif f[0] == 'deferror':
            d['deferror'] = f[1]
        elif f[0] == 'top':
            d['top'].append((f[2], int(f[3]), int(f[4]), f[5], f[6], f[7]))
        elif f[0] == 'target':
            d['targets'][f[1]] = dict(rank=int(f[2]), kind=f[5], role=f[6], symbol=f[7])
        elif f[0] == 'doc':
            d['docs'][f[1]] = dict(present=f[2] == '1', language=f[3], role=f[4], chunks=int(f[5]),
                                   chunk_kinds=f[6] if len(f) > 6 else '',
                                   symbols=[s.split('|')[0] for s in (f[7] if len(f) > 7 else '').split(';') if s])
        elif f[0] == 'cover':
            d.setdefault('covers', []).append(dict(rank=int(f[2]), start=int(f[3]), end=int(f[4]), kind=f[5],
                                                   symbol=f[6]))
        elif f[0] == 'above':
            d['above'] = dict(roles=dict(x.split(':') for x in f[2].split(',') if x),
                              kinds=dict(x.split(':') for x in f[3].split(',') if x),
                              same_file=int(f[4]), median_span=int(f[5]))
        elif f[0] == 'error':
            d['error'] = f[1]
    return d


WINDOWS = ('window',)


def mechanism(diag):
    """Why a codecite search answer is not first, from the untruncated list.

    The answer's own chunk is the smallest covering chunk that is not a
    fixed-size window. Rules apply in order: only windows cover the answer
    (no definition chunk); most chunks above it are tests, docs or examples;
    its own chunk is at least three times longer than the median chunk above
    it; otherwise other source chunks simply share more of the query's terms."""
    covers = (diag or {}).get('covers') or []
    above = (diag or {}).get('above')
    if not covers or not above:
        return None
    own = [c for c in covers if c['kind'] not in WINDOWS]
    if not own:
        return 'granularity:window-only'
    roles = {k: int(v) for k, v in above['roles'].items()}
    n = sum(roles.values())
    if n and (n - roles.get('source', 0)) * 2 >= n:
        return 'role:non-source-above'
    best = min(own, key=lambda c: c['end'] - c['start'])
    if above['median_span'] and best['end'] - best['start'] + 1 >= 3 * above['median_span']:
        return 'length:long-answer-chunk'
    return 'lexical:other-source-shares-more-terms'


def classify(case, tool, op, status, rank, diag, observed):
    """The outcome of one call on one case; see README for the classes."""
    if status in ('unsupported', 'error'):
        return status
    if case['class'] == 'no-result':
        return 'correct-empty' if status == 'empty' else 'false-positive'
    if not case.get('score', True) or case['class'] not in POSITIVE:
        return 'unscored'
    if rank is not None and rank <= LIMIT:
        return 'hit' if rank == 1 else 'rank:in-output'
    if rank is not None:
        return 'rank:below-cut'
    if not diag or diag.get('error'):
        return 'unresolved'
    if tool == 'codegraph':
        if diag.get('file') is False:
            return 'extraction:not-indexed'
        names = diag.get('nodes', [])
        want = last_name(case['query']) if case['class'] in DEFINITION_CLASSES else None
        if not names or (want and want not in names):
            return 'extraction:not-a-definition'
        return 'unresolved'
    docs = list(diag.get('docs', {}).values())
    if not docs or not any(d['present'] for d in docs):
        return 'extraction:not-indexed'
    if op == 'def':
        want = last_name(case['query'])
        if not any(want in d['symbols'] for d in docs):
            return 'extraction:not-a-definition'
        return 'rank:below-cut' if diag.get('deftotal', 0) > len(observed) else 'candidate:lookup-miss'
    top = [tuple(t[:3]) for t in diag.get('top', [])]
    if [tuple(o) for o in observed] != top[:len(observed)] or (not observed and diag.get('total', 0) > 0):
        return 'unresolved'
    ranks = [t['rank'] for t in diag.get('targets', {}).values() if t['rank'] > 0]
    if ranks:
        return 'rank:below-cut'
    if not any(d['chunks'] > 0 for d in docs if d['present']):
        return 'extraction:not-indexed'
    return 'candidate:not-matched'


def codegraph_diag(db, case):
    out = dict(file=False, nodes=[])
    con = sqlite3.connect(f'file:{db}?mode=ro', uri=True)
    try:
        for a in case['answers']:
            if con.execute('SELECT 1 FROM files WHERE path = ?', (a['path'],)).fetchone() or \
               con.execute('SELECT 1 FROM nodes WHERE file_path = ? LIMIT 1', (a['path'],)).fetchone():
                out['file'] = True
            out['nodes'] += [r[0] for r in con.execute(
                'SELECT name FROM nodes WHERE file_path = ? AND start_line <= ? AND end_line >= ? '
                "AND kind NOT IN ('file', 'import', 'module')", (a['path'], a['line'], a['line']))]
    finally:
        con.close()
    return out


def codegraph_spans(db, rows):
    """Widen a node row that printed only its first line to the node's end."""
    con = sqlite3.connect(f'file:{db}?mode=ro', uri=True)
    try:
        out = []
        for path, start, end in rows:
            if end == start:
                r = con.execute('SELECT max(end_line) FROM nodes WHERE file_path = ? AND start_line = ?',
                                (path, start)).fetchone()
                end = max(end, r[0] or end)
            out.append((path, start, end))
        return out
    finally:
        con.close()


def diagnose(args):
    run, src = Path(args.run).resolve(), Path(args.src).resolve()
    corpora, cases = load_corpora(), load_cases()
    check_or_exit(corpora, cases, git_head(src), checkout_lines(src))
    by_id = {c['id']: c for c in cases}
    env = environment(run)
    records = [json.loads(l) for l in (run / 'measure' / 'calls.jsonl').open()]
    warm = [r for r in records if r['sweep'] == 'warmup']
    out = (run / 'diagnose.jsonl').open('w')
    citations = set()
    for r in warm:
        case = by_id[r['case']]
        rows = [tuple(x) for x in r.get('rows', [])]
        db = src / case['repo'] / '.codegraph' / 'codegraph.db'
        if r['tool'] == 'codegraph' and r.get('status') == 'ok' and r['op'] == 'node':
            rows = codegraph_spans(db, rows)
        rank = first_hit(rows, case['answers']) if case['answers'] else None
        diag = None
        positive = case['class'] in POSITIVE and case.get('score', True)
        if positive and r['status'] in ('ok', 'empty') and rank != 1:
            if r['tool'] == 'codecite':
                targets = [f'{a["path"]}:{a["line"]}' for a in case['answers']]
                mode = 'def' if r['op'] == 'def' else 'search'
                cmd = [args.helper, str(run / 'data' / 'codecite' / f'{case["repo"]}.db'), mode,
                       case['query'], case.get('path') or '-', *targets]
                t0 = time.perf_counter()
                proc = subprocess.run(['unshare', '-rn', *cmd], capture_output=True, text=True, env=env)
                diag = parse_candidates(proc.stdout)
                diag['helper_seconds'] = round(time.perf_counter() - t0, 3)
                if proc.returncode != 0 and 'error' not in diag:
                    diag['error'] = proc.stderr.strip()[:200] or f'exit {proc.returncode}'
            else:
                diag = codegraph_diag(db, case)
        outcome = classify(case, r['tool'], r['op'], r['status'], rank, diag, rows[:LIMIT])
        if r['tool'] == 'codecite' and r['status'] == 'ok':
            data = json.loads((run / 'measure' / 'raw' / f'warmup-{r["case"]}-codecite-{r["op"]}.out').read_text())
            citations.update((case['repo'], x['citation']) for x in data['results'])
        out.write(json.dumps(dict(case=r['case'], repo=case['repo'], split=corpora[case['repo']]['split'],
                                  cls=case['class'], tool=r['tool'], op=r['op'], status=r['status'],
                                  rank=rank, outcome=outcome, total=r.get('total'),
                                  top=[f'{p}#L{s}-L{e}' for p, s, e in rows[:LIMIT]],
                                  mechanism=mechanism(diag) if r['tool'] == 'codecite' and r['op'] == 'search'
                                  and outcome.startswith('rank') else None,
                                  diag=summarise_diag(diag))) + '\n')
    out.close()
    # Citation round trips, outside any timed run.
    trips = []
    for repo, citation in sorted(citations):
        proc = subprocess.run(['unshare', '-rn', args.codecite, 'read', repo, citation, '--json'],
                              capture_output=True, text=True, env=env)
        ok = proc.returncode == 0
        if ok:
            try:
                ok = json.loads(proc.stdout).get('citation') == citation
            except ValueError:
                ok = False
        trips.append(dict(repo=repo, citation=citation, ok=ok))
    (run / 'citations.json').write_text(json.dumps(trips, indent=1) + '\n')


def summarise_diag(diag):
    if not diag:
        return None
    keep = {k: diag[k] for k in ('total', 'deftotal', 'deferror', 'error', 'file', 'nodes', 'helper_seconds')
            if k in diag}
    for k in ('targets', 'covers', 'above'):
        if k in diag:
            keep[k] = diag[k]
    if 'docs' in diag:
        keep['docs'] = {k: dict(v, symbols=v['symbols'][:8]) for k, v in diag['docs'].items()}
    if diag.get('top'):
        keep['top'] = [dict(at=f'{t[0]}#L{t[1]}-L{t[2]}', kind=t[3], role=t[4], symbol=t[5]) for t in diag['top'][:5]]
    return keep


# ---------------------------------------------------------------- report

def percentile(values, q):
    """Nearest-rank percentile."""
    s = sorted(values)
    if not s:
        return None
    k = max(0, min(len(s) - 1, -(-q * len(s) // 100) - 1))
    return s[int(k)]


def rank_metrics(ranks):
    n = len(ranks)
    if not n:
        return dict(n=0)
    return {'n': n, 'hit@1': sum(r == 1 for r in ranks if r) / n,
            'hit@10': sum(1 for r in ranks if r and r <= LIMIT) / n,
            'mrr@10': sum(1 / r for r in ranks if r and r <= LIMIT) / n}


def report(args):
    run, out = Path(args.run).resolve(), Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    corpora = load_corpora()
    diag = [json.loads(l) for l in (run / 'diagnose.jsonl').open()]
    calls = [json.loads(l) for l in (run / 'measure' / 'calls.jsonl').open()]
    meta = json.loads((run / 'measure' / 'metadata.json').read_text())
    root = str(Path(args.src).resolve()) if args.src else None

    def scrub(cmd):
        return [Path(c).name if i == 0 else (c.replace(root, '<src>') if root else c) for i, c in enumerate(cmd)]

    with (out / 'calls.jsonl').open('w') as f:
        for c in calls:
            c = dict(c)
            if c.get('argv'):
                c['argv'] = scrub(c['argv'])
            f.write(json.dumps(c) + '\n')
    with (out / 'per-query.jsonl').open('w') as f:
        for d in diag:
            f.write(json.dumps(d) + '\n')
    meta.pop('host', None)
    if isinstance(meta.get('pins'), dict):
        meta['pins'] = [dict(repo=n, commit=c) for n, c in meta['pins'].items()]
    meta['harness_revision_at_report'] = subprocess.check_output(['git', '-C', str(HERE), 'rev-parse', 'HEAD'],
                                                               text=True).strip()
    (out / 'metadata.json').write_text(json.dumps(meta, indent=1) + '\n')
    trips = json.loads((run / 'citations.json').read_text())
    summary = dict(citations=dict(checked=len(trips), resolved=sum(t['ok'] for t in trips)), quality=[], timing=[],
                   outcomes=[])
    groups = defaultdict(list)
    for d in diag:
        groups[(d['split'], d['repo'], d['tool'], d['op'], d['cls'])].append(d)
    for (split, repo, tool, op, cls), ds in sorted(groups.items()):
        scored = [d for d in ds if d['outcome'] not in ('unscored', 'unsupported', 'error', 'correct-empty',
                                                        'false-positive')]
        row = dict(split=split, repo=repo, tool=tool, op=op, cls=cls, cases=len(ds),
                   **rank_metrics([d['rank'] for d in scored]),
                   outcomes=dict(Counter(d['outcome'] for d in ds)))
        summary['quality'].append(row)
    timed = [c for c in calls if c['sweep'] != 'warmup' and 'elapsed_ms' in c]
    by_id = {d['case']: d for d in diag}
    tgroups = defaultdict(list)
    for c in timed:
        d = by_id[c['case']]
        tgroups[(d['split'], d['repo'], c['tool'], c['op'], d['cls'])].append(c)
    for (split, repo, tool, op, cls), cs in sorted(tgroups.items()):
        ms = [c['elapsed_ms'] for c in cs]
        summary['timing'].append(dict(split=split, repo=repo, tool=tool, op=op, cls=cls, samples=len(cs),
                                      queries=len({c['case'] for c in cs}), p50_ms=percentile(ms, 50),
                                      p95_ms=percentile(ms, 95), errors=sum(c['status'] == 'error' for c in cs),
                                      median_stdout_bytes=percentile([c['stdout_bytes'] for c in cs], 50)))
    fc = defaultdict(Counter)
    for d in diag:
        if d['outcome'] not in ('unscored', 'unsupported'):
            fc[(d['split'], d['tool'], d['op'])][(d['repo'], d['outcome'])] += 1
    for (split, tool, op), counter in sorted(fc.items()):
        summary['outcomes'].append(dict(split=split, tool=tool, op=op,
                                        by_repo={f'{r}:{o}': n for (r, o), n in sorted(counter.items())}))
    mech = Counter((d['split'], d['repo'], d['mechanism']) for d in diag if d.get('mechanism'))
    summary['mechanisms'] = [dict(split=a, repo=b, mechanism=c, misses=n) for (a, b, c), n in sorted(mech.items())]
    (out / 'summary.json').write_text(json.dumps(summary, indent=1) + '\n')
    (out / 'summary.md').write_text(markdown(summary, corpora))


def markdown(summary, corpora):
    lines = ['# Results summary', '', 'Generated by `run.py report`. Hit@k and MRR@10 count scored positive cases '
             'only; unsupported operations, infrastructure errors, unscored cases and no-result controls are '
             'listed under outcomes and never counted as misses.', '']
    lines += ['## Quality', '', '| split | repo | tool | op | class | n | Hit@1 | Hit@10 | MRR@10 | outcomes |',
              '| --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: | --- |']
    for r in summary['quality']:
        f = (lambda k: f'{r[k]:.2f}' if k in r else '–')
        oc = ', '.join(f'{k} {v}' for k, v in sorted(r['outcomes'].items()))
        lines.append(f"| {r['split']} | {r['repo']} | {r['tool']} | {r['op']} | {r['cls']} | {r['n']} | "
                     f"{f('hit@1')} | {f('hit@10')} | {f('mrr@10')} | {oc} |")
    lines += ['', '## Timing (startup included, timed rounds only)', '',
              '| split | repo | tool | op | class | samples | queries | p50 ms | p95 ms | errors | median stdout bytes |',
              '| --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    for r in summary['timing']:
        lines.append(f"| {r['split']} | {r['repo']} | {r['tool']} | {r['op']} | {r['cls']} | {r['samples']} | "
                     f"{r['queries']} | {r['p50_ms']:.1f} | {r['p95_ms']:.1f} | {r['errors']} | "
                     f"{r['median_stdout_bytes']} |")
    lines += ['', '## Why codecite search answers are not first', '',
              '| split | repo | mechanism | misses |', '| --- | --- | --- | ---: |']
    lines += [f"| {m['split']} | {m['repo']} | {m['mechanism']} | {m['misses']} |" for m in summary['mechanisms']]
    c = summary['citations']
    lines += ['', f"Citation round trips through `codecite read`: {c['resolved']} of {c['checked']} resolved.", '']
    return '\n'.join(lines)


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest='cmd', required=True)
    v = sub.add_parser('validate')
    v.add_argument('--src', required=True)
    m = sub.add_parser('measure')
    for a in ('--run', '--src', '--codecite'):
        m.add_argument(a, required=True)
    m.add_argument('--rounds', type=int, default=5)
    m.add_argument('--seed', type=int, default=287)
    d = sub.add_parser('diagnose')
    for a in ('--run', '--src', '--codecite', '--helper'):
        d.add_argument(a, required=True)
    r = sub.add_parser('report')
    r.add_argument('--run', required=True)
    r.add_argument('--out', default=str(HERE / 'results'))
    r.add_argument('--src')
    args = p.parse_args()
    if args.cmd == 'validate':
        check_or_exit(load_corpora(), load_cases(), git_head(args.src), checkout_lines(args.src))
        print(f'{len(load_cases())} cases valid')
    elif args.cmd == 'measure':
        args.codecite = str(Path(args.codecite).resolve())
        measure(args)
    elif args.cmd == 'diagnose':
        diagnose(args)
    else:
        report(args)


if __name__ == '__main__':
    main()
