// Settlement-epoch arithmetic. Rewards settle every 10 minutes
// (`weft_primitives::EPOCH_SECONDS`); an epoch index is `floor(ts / 600)` and a
// receipt belongs to an epoch only if its whole [start, end) window lies inside it.

export const EPOCH_SECONDS = 600n;

export function epochOf(unixSeconds: bigint): bigint {
  return unixSeconds / EPOCH_SECONDS;
}

export function epochRange(epoch: bigint): { start: bigint; end: bigint } {
  return { start: epoch * EPOCH_SECONDS, end: (epoch + 1n) * EPOCH_SECONDS };
}

/** A window is in-epoch iff start < end and both endpoints fall in the same epoch. */
export function windowInEpoch(epoch: bigint, windowStart: bigint, windowEnd: bigint): boolean {
  if (windowEnd <= windowStart) return false;
  const { start, end } = epochRange(epoch);
  return windowStart >= start && windowEnd <= end;
}
