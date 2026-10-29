"""Per sample: which messages carry the repeated-parameter rule text. Usage: python3 -I exposure.py <log dir>"""
import sys, zipfile, json, glob, os
for f in sorted(glob.glob(os.path.join(sys.argv[1], '**/*.eval'), recursive=True)):
    z = zipfile.ZipFile(f)
    model = json.loads(z.read('header.json'))['eval']['model'].split('/')[-1]
    for n in sorted(x for x in z.namelist() if x.startswith('samples/')):
        s = json.loads(z.read(n))
        roles = sorted({(m['role'], (m.get('function') or '')) for m in s['messages'] if 'more than once' in json.dumps(m.get('content'))+json.dumps(m.get('tool_calls'))})
        print(model, n.split('/')[1], [sc.get('value') for sc in s['scores'].values()], roles)
