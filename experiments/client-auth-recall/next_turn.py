"""For Sonnet codecite runs: what Sonnet said right after a tool result carrying the rule."""
import sys, zipfile, json, glob, os
for f in glob.glob(os.path.join(sys.argv[1], '**/*.eval'), recursive=True):
    z = zipfile.ZipFile(f)
    if 'sonnet' not in json.loads(z.read('header.json'))['eval']['model']: continue
    for n in sorted(x for x in z.namelist() if x.startswith('samples/codecite')):
        msgs = json.loads(z.read(n))['messages']
        for i, m in enumerate(msgs):
            if m['role'] == 'tool' and 'more than once' in json.dumps(m.get('content')):
                nxt = next((x for x in msgs[i+1:] if x['role'] == 'assistant'), None)
                txt = nxt['content'] if isinstance(nxt['content'], str) else ' '.join(c.get('text', '') or c.get('reasoning', '') for c in nxt['content'] if isinstance(c, dict))
                print('==', n, '\n', txt[:900])
                break
