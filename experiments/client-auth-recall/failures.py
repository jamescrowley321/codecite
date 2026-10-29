"""Tally of the hidden tests each model and arm fails. Usage: python3 -I failures.py <log dirs>..."""
import sys, zipfile, json, glob, os, re, collections
c = collections.Counter()
for d in sys.argv[1:]:
    for f in glob.glob(os.path.join(d, '**/*.eval'), recursive=True):
        z = zipfile.ZipFile(f)
        model = json.loads(z.read('header.json'))['eval']['model'].split('/')[-1]
        for n in (x for x in z.namelist() if x.startswith('samples/')):
            s = json.loads(z.read(n)); arm = n.split('/')[1].split('_')[0]
            for sc in s['scores'].values():
                tail = (sc.get('metadata') or {}).get('tests_tail') or ''
                fails = sorted(set(re.findall(r'FAILED \S+::(\w+)', tail))) or (['-'] if sc.get('value') else ['?'])
                c[(model, arm, tuple(fails))] += 1
for k, v in sorted(c.items()): print(v, *k)
