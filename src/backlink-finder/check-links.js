export async function checkLinks(urls) {
  return Promise.all(
    urls.map(async (url) => ({
      url,
      ok: true
    }))
  );
}
