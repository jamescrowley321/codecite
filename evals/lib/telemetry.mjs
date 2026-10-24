// Token and cost usage. A number an agent didn't report is null (unknown),
// never 0, and unknown never becomes known by adding to it.

export const USAGE_FIELDS = ['input', 'output', 'cacheRead', 'cacheWrite', 'totalTokens', 'costUsd'];

// Marks the accumulator that has had nothing added yet, so the first
// addUsage takes its values as they are.
const EMPTY = Symbol('emptyUsage');

/** A Usage with every field unknown, to start summing from. */
export function emptyUsage() {
  const u = Object.fromEntries(USAGE_FIELDS.map((f) => [f, null]));
  Object.defineProperty(u, EMPTY, { value: true });
  return u;
}

/**
 * Sum two Usages field by field. Adding to emptyUsage() takes `b` as is;
 * after that, null on either side gives null.
 */
export function addUsage(a, b) {
  if (a[EMPTY]) {
    const u = Object.fromEntries(USAGE_FIELDS.map((f) => [f, b[f] ?? null]));
    if (b[EMPTY]) Object.defineProperty(u, EMPTY, { value: true });
    return u;
  }
  return Object.fromEntries(
    USAGE_FIELDS.map((f) => [f, a[f] == null || b[f] == null ? null : a[f] + b[f]]),
  );
}
