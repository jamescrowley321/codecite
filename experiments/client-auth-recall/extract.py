"""Every codecite call in client-auth-uri logs, with the run outcome. Usage: python3 -I extract.py <log dirs>... > calls.json"""
import sys, zipfile, json, glob, os
rows = []
for d in sys.argv[1:]:
    for f in sorted(glob.glob(os.path.join(d, '**/*.eval'), recursive=True)):
        z = zipfile.ZipFile(f)
        model = json.loads(z.read('header.json'))['eval']['model'].split('/')[-1]
        for n in sorted(x for x in z.namelist() if x.startswith('samples/codecite')):
            s = json.loads(z.read(n))
            passed = [sc.get('value') for sc in s['scores'].values()][0]
            for m in s['messages']:
                for tc in m.get('tool_calls') or []:
                    if tc['function'].startswith('mcp__codecite__'):
                        rows.append({'run': os.path.basename(d), 'model': model, 'sample': n.split('/')[1],
                                     'passed': passed, 'tool': tc['function'].split('__')[-1], 'args': tc['arguments']})
json.dump(rows, sys.stdout, indent=1)
