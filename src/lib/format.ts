/** Human-readable byte size, preserving the app's existing KB / MB labels. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${Math.round(n)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = n / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

/** Locale date and time, e.g. "Jan 5, 2026, 3:04 PM". */
export function formatDateTime(at: number): string {
  return new Date(at).toLocaleString();
}

/**
 * Relative time in minutes, hours or days via `Intl.RelativeTimeFormat`.
 * `now` defaults to the current time; it is a parameter so callers and checks
 * can pin the same instant.
 */
export function formatRelativeTime(at: number, now: number = Date.now()): string {
  const seconds = (at - now) / 1000;
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const magnitude = Math.abs(seconds);
  if (magnitude < 3600) return formatter.format(Math.round(seconds / 60), "minute");
  if (magnitude < 86_400) return formatter.format(Math.round(seconds / 3600), "hour");
  return formatter.format(Math.round(seconds / 86_400), "day");
}

/** A digit string spaced into readable groups of three, e.g. "123 456". */
export function groupDigits(code: string): string {
  return code.replace(/(.{3})(?=.)/g, "$1 ");
}
