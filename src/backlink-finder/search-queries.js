export function generateSearchQueries(domain) {
  const cleanDomain = normalizeDomain(domain);

  const queries = [
    `"${cleanDomain}" -site:${cleanDomain}`,
    `"https://${cleanDomain}" -site:${cleanDomain}`,
    `"http://${cleanDomain}" -site:${cleanDomain}`,
    `"www.${cleanDomain}" -site:${cleanDomain}`,
  ];

  return queries.flatMap((query) => [
    {
      engine: "google",
      query,
      url: createGoogleSearchUrl(query),
    },
    {
      engine: "bing",
      query,
      url: createBingSearchUrl(query),
    },
  ]);
}

function createGoogleSearchUrl(query) {
  return `https://www.google.com/search?q=${encodeURIComponent(query)}`;
}

function createBingSearchUrl(query) {
  return `https://www.bing.com/search?q=${encodeURIComponent(query)}`;
}

function normalizeDomain(domain) {
  return domain
    .trim()
    .toLowerCase()
    .replace(/^https?:\/\//, "")
    .replace(/^www\./, "")
    .replace(/\/.*$/, "");
}