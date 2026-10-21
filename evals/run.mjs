#!/usr/bin/env node
// The A/B eval harness CLI. Phases: compose (no secrets), call, score.
// See evals/README.md.

import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { parseArgs } from 'node:util';
import { expandRuns, freezePlan, loadPlan, loadTask, roundsDir } from './lib/plan.mjs';
import { prepareRound, REPO_ROOT } from './lib/workspace.mjs';

const USAGE = `usage:
  node evals/run.mjs compose --plan <plan.json> [--round <id>] [--codecite-bin <path>] [--dry-run]
  node evals/run.mjs call --round <id>
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

function compose(argv) {
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
  loadTask(spec.task);
  const dir = roundsDir();
  if (values['dry-run']) {
    printCounts(spec);
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

const [command, ...rest] = process.argv.slice(2);
try {
  if (command === 'compose') compose(rest);
  else if (command === 'call' || command === 'score') {
    console.error(`${command}: not implemented yet`);
    process.exit(2);
  } else {
    console.error(USAGE);
    process.exit(2);
  }
} catch (e) {
  console.error(e.message);
  process.exit(1);
}
