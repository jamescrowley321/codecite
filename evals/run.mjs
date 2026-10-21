#!/usr/bin/env node
// The A/B eval harness CLI. Phases: compose (no secrets), call, score.
// See evals/README.md.

import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { parseArgs } from 'node:util';
import { callRound, redact } from './lib/call.mjs';
import { estimateUsd, modelPrices } from './lib/openrouter.mjs';
import { expandRuns, freezePlan, loadPlan, loadTask, roundsDir } from './lib/plan.mjs';
import { readSecrets } from './lib/secrets.mjs';
import { prepareRound, REPO_ROOT } from './lib/workspace.mjs';

const USAGE = `usage:
  node evals/run.mjs compose --plan <plan.json> [--round <id>] [--codecite-bin <path>] [--dry-run]
  node evals/run.mjs call --round <id> [--retry-fatal]
  node evals/run.mjs score --round <id>`;

function git(...args) {
  const r = spawnSync('git', ['-C', REPO_ROOT, ...args], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`git ${args.join(' ')}: ${r.stderr.trim()}`);
  return r.stdout.trim();
}

function printCounts(plan) {
  const runs = expandRuns(plan);
  const counts = new Map();
  for (const r of runs) {
    const key = `${r.agent} ${r.model}`;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  for (const [pair, n] of counts) console.log(`${pair}: ${n} runs`);
  console.log(`total: ${runs.length} runs`);
}

// The OpenRouter estimate needs live prices (no key); it is advisory here.
async function printEstimate(plan) {
  try {
    const usd = estimateUsd(plan, { prices: await modelPrices() });
    console.log(`estimate: $${usd.toFixed(2)} on OpenRouter (Claude runs counted as $0)`);
  } catch (e) {
    console.log(`estimate: unavailable (${e.message})`);
  }
}

async function compose(argv) {
  const { values } = parseArgs({
    args: argv,
    options: {
      plan: { type: 'string' },
      round: { type: 'string' },
      'codecite-bin': { type: 'string' },
      'dry-run': { type: 'boolean', default: false },
    },
  });
  if (!values.plan) throw new Error(`compose needs --plan\n${USAGE}`);
  const spec = loadPlan(values.plan);
  const task = loadTask(spec.task);
  if (!spec.repos[task.corpus.repo]) throw new Error(`plan: repos has no path for "${task.corpus.repo}"`);
  const dir = roundsDir();
  if (values['dry-run']) {
    printCounts(spec);
    await printEstimate(spec);
    console.log('dry run: nothing frozen, nothing spent');
    return;
  }
  const roundId = values.round ?? `round-${new Date().toISOString().replace(/[-:]/g, '').replace(/\..*$/, '')}`;
  const plan = freezePlan(spec, {
    codeciteCommit: git('rev-parse', 'HEAD'),
    codeciteDirty: git('status', '--porcelain') !== '',
    roundId,
  });
  const round = prepareRound(plan, join(dir, roundId), { codeciteBin: values['codecite-bin'] });
  printCounts(plan);
  console.log(`round ${roundId} prepared at ${round.dir}`);
}

// Loaded by `call`; every error printed after that is redacted with it.
let secrets = null;

async function call(argv) {
  const { values } = parseArgs({
    args: argv,
    options: { round: { type: 'string' }, 'retry-fatal': { type: 'boolean', default: false } },
  });
  if (!values.round) throw new Error(`call needs --round\n${USAGE}`);
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(values.round)) throw new Error(`call: bad round id "${values.round}"`);
  secrets = readSecrets();
  const summary = await callRound(join(roundsDir(), values.round), { secrets, retryFatal: values['retry-fatal'] });
  console.log(`ran ${summary.ran}, skipped ${summary.skipped}`);
  if (summary.stopped) {
    console.error(redact(`round stopped (${summary.stopped}): ${summary.reason}`, secrets));
    process.exit(1);
  }
}

const [command, ...rest] = process.argv.slice(2);
try {
  if (command === 'compose') await compose(rest);
  else if (command === 'call') await call(rest);
  else if (command === 'score') {
    console.error(`${command}: not implemented yet`);
    process.exit(2);
  } else {
    console.error(USAGE);
    process.exit(2);
  }
} catch (e) {
  console.error(redact(e.message, secrets));
  process.exit(1);
}
