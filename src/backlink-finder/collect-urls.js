export function collectUrls(results) {
  return results.map((item) => item.url).filter(Boolean);
}
