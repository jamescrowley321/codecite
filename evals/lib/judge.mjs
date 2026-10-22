// LLM judge: grade each scored run's diff against the task, blind to its arm,
// and record judge.json beside score.json. Reported in the scorecard, never
// part of the verdict.

import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { loadRound, redact } from './call.mjs';
import { expandRuns } from './plan.mjs';
import { requireSecret } from './secrets.mjs';
import { SCORED } from './score.mjs';

const API = 'https://openrouter.ai/api/v1/chat/completions';
export const JUDGE_MODEL = 'anthropic/claude-opus-5-5';
export const RUBRIC_VERSION = 1;
export const MAX_DIFF_LINES = 400;
export const CRITERIA = ['instructions', 'library', 'hygiene'];

const SYSTEM = `You grade one solution to a coding task. You see the task as the developer saw it and the unified diff of what they changed under app/. Grade only what the diff shows.

Score each criterion from 1 (poor) to 5 (excellent), with a one-sentence reason:

- instructions: every requirement of the task is met, done the way the task says (the named middleware, the named dependency, settings from the named environment variables, /health open, no new packages).
- library: the solution uses the library as intended (its middleware, its dependency, its settings helpers) rather than reimplementing token validation or claim handling by hand.
- hygiene: the change is small and focused; no dead code, debugging prints, commented-out code, hard-coded settings or needless files.

Answer with JSON only.`;

const SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: CRITERIA,
  properties: Object.fromEntries(
    CRITERIA.map((c) => [
      c,
      {
        type: 'object',
        additionalProperties: false,
        required: ['score', 'reason'],
        properties: { score: { type: 'integer', minimum: 1, maximum: 5 }, reason: { type: 'string' } },
      },
    ]),
  ),
};

/** The run's change under app/ as a unified diff (at most MAX_DIFF_LINES lines), with neutral paths. */
export function runDiff(task, runDir) {
  const start = resolve(task.dir, task.project, 'app');
  const final = join(runDir, 'project', 'app');
  // The agent wrote `final`, and a symlink it planted there can point at any
  // host file, which a diff would send to the judge. A solution never needs
  // one, so the run goes unjudged; --no-dereference backs that up.
  const links = readdirSync(final, { recursive: true, withFileTypes: true }).filter((e) => e.isSymbolicLink());
  if (links.length) throw new Error(`judge: app/ holds symlinks, which are never diffed: ${links.map((e) => relative(final, join(e.parentPath, e.name))).join(', ')}`);
  const r = spawnSync('diff', ['-ruN', '--no-dereference', '--exclude=__pycache__', '--exclude=*.pyc', start, final], { encoding: 'utf8', maxBuffer: 16 << 20 });
  if (r.status !== 0 && r.status !== 1) throw new Error(`judge: diff failed: ${r.stderr.trim()}`);
  // Host paths become a/app and b/app; the `diff` command lines and the file
  // times go, since they say nothing about the change.
  const lines = r.stdout
    .replaceAll(start, 'a/app')
    .replaceAll(final, 'b/app')
    .split('\n')
    .filter((l) => !l.startsWith('diff '))
    .map((l) => (/^(---|\+\+\+) /.test(l) ? l.split('\t')[0] : l));
  return lines.length > MAX_DIFF_LINES ? `${lines.slice(0, MAX_DIFF_LINES).join('\n')}\n[diff cut at ${MAX_DIFF_LINES} lines]` : lines.join('\n');
}

/** Parse and check the judge's answer: {instructions, library, hygiene}, each {score 1-5, reason}. */
export function parseGrade(content) {
  const g = JSON.parse(String(content).replace(/^```(?:json)?\s*|\s*```$/g, ''));
  for (const c of CRITERIA) {
    const s = g?.[c]?.score;
    if (!Number.isInteger(s) || s < 1 || s > 5) throw new Error(`judge: ${c} score is not 1-5`);
  }
  return Object.fromEntries(CRITERIA.map((c) => [c, { score: g[c].score, reason: String(g[c].reason ?? '') }]));
}

async function askJudge({ key, model, prompt, diff, fetchImpl = fetch }) {
  const res = await fetchImpl(API, {
    method: 'POST',
    headers: { Authorization: `Bearer ${key}`, 'Content-Type': 'application/json', 'X-Title': 'eval-judge' },
    body: JSON.stringify({
      model,
      temperature: 0,
      max_tokens: 800,
      response_format: { type: 'json_schema', json_schema: { name: 'grade', strict: true, schema: SCHEMA } },
      messages: [
        { role: 'system', content: SYSTEM },
        { role: 'user', content: `## Task\n\n${prompt}\n\n## Diff\n\n\`\`\`diff\n${diff || '(no change)'}\n\`\`\`` },
      ],
    }),
    signal: AbortSignal.timeout(120_000),
  });
  const body = await res.json().catch(() => null);
  if (!res.ok) throw new Error(`judge: HTTP ${res.status}: ${body?.error?.message ?? ''}`.trim());
  const content = body?.choices?.[0]?.message?.content;
  if (!content) throw new Error('judge: empty answer');
  return { grade: parseGrade(content), costUsd: typeof body?.usage?.cost === 'number' ? body.usage.cost : null };
}

/**
 * Judge every scored run of a round that has no judge.json yet (all of them
 * with `rejudge`), `concurrency` at once. Answers are cached by diff hash in
 * <round>/judge-cache, so identical diffs and re-runs cost nothing. A failed
 * call leaves {error} in judge.json and the run unjudged; it never fails the phase.
 */
export async function judgeRound(roundDir, { secrets, model = JUDGE_MODEL, concurrency = 16, rejudge = false, log = console.log, fetchImpl } = {}) {
  const key = requireSecret(secrets, 'openrouter');
  const round = loadRound(roundDir);
  const task = round.plan.taskDef;
  const prompt = task.promptText.replaceAll('{identity_model}', '/work/identity-model');
  const cacheDir = join(round.dir, 'judge-cache');
  mkdirSync(cacheDir, { recursive: true });
  const queue = expandRuns(round.plan).filter((run) => {
    const dir = join(round.dir, 'runs', run.id);
    if (!rejudge && existsSync(join(dir, 'judge.json')) && !JSON.parse(readFileSync(join(dir, 'judge.json'), 'utf8')).error) return false;
    try {
      return SCORED.has(JSON.parse(readFileSync(join(dir, 'result.json'), 'utf8')).status) && existsSync(join(dir, 'project'));
    } catch {
      return false;
    }
  });
  let judged = 0;
  let failed = 0;
  const one = async (run) => {
    const dir = join(round.dir, 'runs', run.id);
    let diffSha = null;
    let out;
    try {
      const diff = runDiff(task, dir);
      diffSha = createHash('sha256').update(`${RUBRIC_VERSION}\n${model}\n${diff}`).digest('hex');
      const cached = join(cacheDir, `${diffSha}.json`);
      const hit = existsSync(cached) ? JSON.parse(readFileSync(cached, 'utf8')) : null;
      const { grade, costUsd } = hit ?? (await askJudge({ key, model, prompt, diff, fetchImpl }));
      if (!hit) writeFileSync(cached, `${JSON.stringify({ grade, costUsd }, null, 2)}\n`);
      out = { model, rubricVersion: RUBRIC_VERSION, diffSha, diffLines: diff.split('\n').length, cached: Boolean(hit), costUsd: hit ? 0 : costUsd, ...grade, judgedAt: new Date().toISOString() };
      judged++;
    } catch (e) {
      out = { model, diffSha, error: redact(e.message, secrets), judgedAt: new Date().toISOString() };
      failed++;
      log(`${run.id} unjudged: ${out.error}`);
    }
    writeFileSync(join(dir, 'judge.json'), `${JSON.stringify(out, null, 2)}\n`);
  };
  await Promise.all(
    Array.from({ length: Math.max(1, concurrency) }, async () => {
      for (let run = queue.shift(); run; run = queue.shift()) await one(run);
    }),
  );
  return { judged, failed };
}
