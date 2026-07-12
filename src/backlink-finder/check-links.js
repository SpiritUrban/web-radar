import * as cheerio from "cheerio";

const REQUEST_TIMEOUT_MS = 15_000;
const CONCURRENCY = 5;

export async function checkLinks({
  urls,
  targetDomain,
}) {
  const normalizedTarget = normalizeHostname(targetDomain);
  const results = [];

  for (let index = 0; index < urls.length; index += CONCURRENCY) {
    const batch = urls.slice(index, index + CONCURRENCY);

    const batchResults = await Promise.allSettled(
      batch.map((url) =>
        checkPage({
          url,
          targetDomain: normalizedTarget,
        }),
      ),
    );

    for (const result of batchResults) {
      if (result.status === "fulfilled" && result.value) {
        results.push(result.value);
      }

      if (result.status === "rejected") {
        console.error(result.reason.message);
      }
    }
  }

  return results;
}

async function checkPage({
  url,
  targetDomain,
}) {
  console.log(`Checking: ${url}`);

  const controller = new AbortController();

  const timeout = setTimeout(() => {
    controller.abort();
  }, REQUEST_TIMEOUT_MS);

  try {
    const response = await fetch(url, {
      signal: controller.signal,
      redirect: "follow",
      headers: {
        "user-agent":
          "Mozilla/5.0 (compatible; WebRadar/0.1; backlink-checker)",
        accept:
          "text/html,application/xhtml+xml",
      },
    });

    if (!response.ok) {
      throw new Error(
        `HTTP ${response.status}: ${url}`,
      );
    }

    const contentType =
      response.headers.get("content-type") ?? "";

    if (!contentType.includes("text/html")) {
      console.log(`Skipped non-HTML: ${url}`);
      return null;
    }

    const html = await response.text();
    const $ = cheerio.load(html);

    const links = [];

    $("a[href]").each((_, element) => {
      const rawHref = $(element).attr("href");

      if (!rawHref) {
        return;
      }

      let resolvedUrl;

      try {
        resolvedUrl = new URL(
          rawHref,
          response.url,
        );
      } catch {
        return;
      }

      const linkHostname = normalizeHostname(
        resolvedUrl.hostname,
      );

      if (!isTargetDomain(linkHostname, targetDomain)) {
        return;
      }

      const rel = (
        $(element).attr("rel") ?? ""
      )
        .toLowerCase()
        .split(/\s+/)
        .filter(Boolean);

      links.push({
        targetUrl: resolvedUrl.href,
        anchor: $(element)
          .text()
          .replace(/\s+/g, " ")
          .trim(),
        rel,
        nofollow: rel.includes("nofollow"),
        sponsored: rel.includes("sponsored"),
        ugc: rel.includes("ugc"),
      });
    });

    if (links.length === 0) {
      return null;
    }

    return {
      sourceUrl: response.url,
      sourceTitle: $("title").first().text().trim(),
      statusCode: response.status,
      linkCount: links.length,
      links,
    };
  } catch (error) {
    if (error.name === "AbortError") {
      throw new Error(`Timeout: ${url}`);
    }

    throw new Error(
      `Failed to check ${url}: ${error.message}`,
    );
  } finally {
    clearTimeout(timeout);
  }
}

function normalizeHostname(value) {
  return value
    .trim()
    .toLowerCase()
    .replace(/^https?:\/\//, "")
    .replace(/^www\./, "")
    .replace(/\/.*$/, "");
}

function isTargetDomain(hostname, targetDomain) {
  return (
    hostname === targetDomain ||
    hostname.endsWith(`.${targetDomain}`)
  );
}