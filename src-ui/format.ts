/** Formatting helpers shared by every view. Ukrainian units, no libraries. */

const BYTE_UNITS = ["Б", "КБ", "МБ", "ГБ", "ТБ"];

export function formatBytes(bytes: number | null | undefined): string {
  if (!bytes || bytes <= 0) return "—";
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit >= 3 ? 1 : 0)} ${BYTE_UNITS[unit]}`;
}

export function formatCount(value: number | null | undefined): string {
  if (value === null || value === undefined) return "—";
  return value.toLocaleString("uk-UA");
}

export function formatDuration(seconds: number): string {
  if (!seconds || seconds < 0) return "—";
  if (seconds < 60) return `${Math.round(seconds)} с`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} хв ${Math.round(seconds % 60)} с`;
  return `${Math.floor(seconds / 3600)} год ${Math.floor((seconds % 3600) / 60)} хв`;
}

/**
 * Ukrainian plural: 1 домен, 2 домени, 5 доменів.
 *
 * `${n} доменів` is wrong for 2–4 and reads like machine output.
 */
export function plural(count: number, one: string, few: string, many: string): string {
  const mod100 = Math.abs(count) % 100;
  const mod10 = mod100 % 10;
  if (mod100 >= 11 && mod100 <= 14) return many;
  if (mod10 === 1) return one;
  if (mod10 >= 2 && mod10 <= 4) return few;
  return many;
}

/** Metric names as their owners spell them. */
export function metricLabel(metric: string): string {
  return metric.toLowerCase() === "harmonic" ? "Harmonic" : "PageRank";
}

/** Ranks span many orders of magnitude, so plain decimals are unreadable. */
export function formatRank(rank: number | null | undefined): string {
  if (rank === null || rank === undefined || rank === 0) return "—";
  if (rank >= 0.001) return rank.toFixed(4);
  return rank.toExponential(3).replace("e", "e");
}

export function formatDateTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? "—" : date.toLocaleString("uk-UA");
}

/** `example.com` from anything a person might paste. */
export function cleanDomain(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/^[a-z]+:\/\//, "")
    .replace(/^www\./, "")
    .replace(/[/?#].*$/, "")
    .replace(/:\d+$/, "")
    .replace(/\.+$/, "");
}

export function deriveBrand(domain: string): string {
  const label = cleanDomain(domain).split(".")[0] ?? "";
  return label
    .split(/[-_]/)
    .filter(Boolean)
    .map((word) => word[0].toUpperCase() + word.slice(1))
    .join(" ");
}

/** Basename of a path, on either platform's separator. */
export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}
