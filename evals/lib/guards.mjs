// Failure guards: what a failed agent call means for the round, and a
// circuit breaker for runaway failure.

/**
 * Provider statuses that mean the round's setup is wrong: stop at once.
 * A 400 is fatal only before any turn; after one it describes the run.
 */
export const FATAL_STATUSES = new Set([400, 401, 402, 404]);

/**
 * What a call's outcome means:
 * - "fatal": provider status 401/402/404, or 400 before any turn (turns 0
 *   or unknown), or the adapter rejected the
 *   run (`rejectReason`, e.g. leaked user config). The round stops.
 * - "infra": a rate or usage limit (`rateLimited`) at any turn, provider
 *   status >= 500 before any turn, a run that ended on a provider error
 *   (`endedOnError`, any status or none but a 400 after a turn) at any turn
 *   count, or the harness could not spawn the agent (`spawnError`). The run
 *   is retried; after the attempts run out it is recorded infra_error.
 * - "agent": everything else, including success.
 * @param {{providerStatus: number|null, turns: number|null, rejectReason?: string|null, spawnError?: string|null, endedOnError?: boolean, rateLimited?: boolean}} r
 * @returns {"fatal"|"infra"|"agent"}
 */
export function classify(r) {
  if (r.rejectReason) return 'fatal';
  if (r.providerStatus != null && FATAL_STATUSES.has(r.providerStatus)) {
    if (!(r.providerStatus === 400 && r.turns > 0)) return 'fatal';
  }
  // A rate or usage limit (a 429, Claude's five-hour limit) is the
  // provider's, not the agent's, however far the run got: redo it.
  if (r.rateLimited) return 'infra';
  if (r.spawnError) return 'infra';
  if (r.providerStatus != null && r.providerStatus >= 500 && r.turns === 0) return 'infra';
  // A run the provider ended (pi's stopReason "error": "Provider
  // finish_reason: error", an empty response; Claude's is_error result) is
  // the provider's fault at any turn count: redo it. A 400 after a turn
  // still describes the run (e.g. the agent overfilled its context).
  if (r.endedOnError && !(r.providerStatus === 400 && r.turns > 0)) return 'infra';
  return 'agent';
}

/** Trips once at least `min` calls are recorded and the failed share is >= `ratio`. */
export class CircuitBreaker {
  constructor({ min = 10, ratio = 0.8 } = {}) {
    this.min = min;
    this.ratio = ratio;
    this.calls = 0;
    this.failed = 0;
  }

  /** @param {boolean} ok */
  record(ok) {
    this.calls++;
    if (!ok) this.failed++;
  }

  get tripped() {
    return this.calls >= this.min && this.failed / this.calls >= this.ratio;
  }
}
