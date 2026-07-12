export function ensureArray(value) {
  return Array.isArray(value) ? value : [value];
}
