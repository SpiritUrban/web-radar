#!/usr/bin/env node
/**
 * Build `site/public/download-manifest.json` from the GitHub release.
 *
 * Rules this encodes, each paid for by a real breakage:
 *  - never hardcode asset names: Tauri names bundles after `productName` and
 *    GitHub replaces spaces with dots (`Web Radar` → `Web.Radar_0.3.0_x64-setup.exe`);
 *  - detect the platform by file extension, not by a word in the name
 *    (`.rpm` and `.app.tar.gz` contain no platform word);
 *  - drop `.sig` and `latest.json` — they are not downloads;
 *  - when there is no release yet, emit an empty asset list and let the page
 *    link to the releases page. Never invent a URL that 404s.
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

for (const stream of [process.stdout, process.stderr]) {
  stream.setDefaultEncoding?.("utf8");
}

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OWNER = "SpiritUrban";
const REPO = "web-radar";
const OUT = resolve(ROOT, "site/public/download-manifest.json");

export function classify(name) {
  const lower = name.toLowerCase();
  let platform = "windows";
  if (
    lower.includes("macos") || lower.includes("darwin") ||
    lower.endsWith(".dmg") || lower.endsWith(".app.tar.gz")
  ) {
    platform = "macos";
  } else if (
    lower.includes("linux") || lower.endsWith(".appimage") ||
    lower.endsWith(".deb") || lower.endsWith(".rpm")
  ) {
    platform = "linux";
  }
  const architecture =
    lower.includes("arm64") || lower.includes("aarch64") ? "arm64" : "x64";
  const kind = lower.endsWith(".msi") ? "msi"
    : lower.endsWith(".exe") ? "exe"
    : lower.endsWith(".dmg") ? "dmg"
    : lower.endsWith(".app.tar.gz") ? "app"
    : lower.endsWith(".appimage") ? "appimage"
    : lower.endsWith(".deb") ? "deb"
    : lower.endsWith(".rpm") ? "rpm"
    : "other";
  return { platform, architecture, kind };
}

export function isDownloadable(name) {
  const lower = name.toLowerCase();
  return !lower.endsWith(".sig") && lower !== "latest.json";
}

async function main() {
  const ref = process.env.GITHUB_REF_NAME ?? "";
  const isTag = /^v\d+\.\d+\.\d+$/.test(ref);
  // A tag deploy must publish the release it was cut from, not whatever
  // `latest` resolves to at that second.
  const apiUrl = isTag
    ? `https://api.github.com/repos/${OWNER}/${REPO}/releases/tags/${ref}`
    : `https://api.github.com/repos/${OWNER}/${REPO}/releases/latest`;

  const headers = { "User-Agent": `${REPO}-site-builder`, Accept: "application/vnd.github+json" };
  // Unauthenticated API calls are limited to 60/hour per IP, then 403.
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;

  const fallbackVersion = JSON.parse(readFileSync(resolve(ROOT, "package.json"), "utf8")).version;
  let manifest = {
    version: isTag ? ref.slice(1) : fallbackVersion,
    tag: isTag ? ref : null,
    releaseUrl: `https://github.com/${OWNER}/${REPO}/releases`,
    publishedAt: null,
    assets: [],
    generatedAt: new Date().toISOString(),
  };

  try {
    const response = await fetch(apiUrl, { headers });
    if (response.ok) {
      const release = await response.json();
      manifest = {
        version: (release.tag_name ?? "").replace(/^v/, "") || manifest.version,
        tag: release.tag_name ?? null,
        releaseUrl: release.html_url ?? manifest.releaseUrl,
        publishedAt: release.published_at ?? null,
        assets: (release.assets ?? [])
          .filter((asset) => isDownloadable(asset.name))
          .map((asset) => ({
            ...classify(asset.name),
            fileName: asset.name,
            sizeBytes: asset.size,
            downloadUrl: asset.browser_download_url,
          })),
        generatedAt: manifest.generatedAt,
      };
    } else {
      console.log(`No usable release (${response.status} from ${apiUrl}) — publishing an empty asset list.`);
    }
  } catch (cause) {
    console.log(`Release lookup failed (${cause}) — publishing an empty asset list.`);
  }

  mkdirSync(dirname(OUT), { recursive: true });
  writeFileSync(OUT, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`ref=${ref || "(none)"} -> version ${manifest.version}, ${manifest.assets.length} assets`);
}

// Importable for tests, runnable as a script.
if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  await main();
}
