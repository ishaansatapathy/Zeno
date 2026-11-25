// Sizes as a person reads them. 1024-based, as the API says them in its own messages, so the page and
// a `413` message never disagree about what a gigabyte is.

const UNITS = ["bytes", "KB", "MB", "GB", "TB"] as const;

/** `1536 * 1024 * 1024` is "1.5 GB". A whole number of bytes stays whole; everything above has one decimal. */
export function formatBytes(bytes: number): string {
  let value = Math.max(0, Number.isFinite(bytes) ? bytes : 0);
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${Math.floor(value)} ${UNITS[0]}` : `${value.toFixed(1)} ${UNITS[unit]}`;
}

/** How much of the allowance is used, as a whole percent from 0 to 100. Any use at all shows as at least 1. */
export function percentUsed(used: number, quota: number): number {
  if (!(used > 0)) return 0;
  if (!(quota > 0)) return 100;
  return Math.min(100, Math.max(1, Math.round((used / quota) * 100)));
}
