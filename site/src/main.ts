/**
 * Renders the download buttons from `download-manifest.json`, which CI
 * regenerates from the GitHub release on every deploy.
 *
 * Asset names and the version are never written here: Tauri decides them and
 * GitHub rewrites them, so anything hardcoded eventually points at a 404.
 */

interface Asset {
  platform: "windows" | "macos" | "linux";
  architecture: "x64" | "arm64";
  kind: string;
  fileName: string;
  sizeBytes: number;
  downloadUrl: string;
}

interface Manifest {
  version: string;
  tag: string | null;
  releaseUrl: string;
  publishedAt: string | null;
  assets: Asset[];
}

/** A card is matched by platform **and** architecture **and** file kind: only
 * platform would make the “MSI” card link to the `.exe`. */
const CARDS: { title: string; note: string; platform: Asset["platform"]; architecture: Asset["architecture"]; kinds: string[] }[] = [
  { title: "Windows", note: "інсталятор .exe", platform: "windows", architecture: "x64", kinds: ["exe"] },
  { title: "Windows", note: "пакет .msi", platform: "windows", architecture: "x64", kinds: ["msi"] },
  { title: "macOS", note: "Apple Silicon", platform: "macos", architecture: "arm64", kinds: ["dmg"] },
  { title: "macOS", note: "Intel", platform: "macos", architecture: "x64", kinds: ["dmg"] },
  { title: "Linux", note: "AppImage", platform: "linux", architecture: "x64", kinds: ["appimage"] },
  { title: "Linux", note: "пакет .deb", platform: "linux", architecture: "x64", kinds: ["deb"] },
];

function formatBytes(bytes: number): string {
  if (!bytes) return "";
  const units = ["Б", "КБ", "МБ", "ГБ"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit > 1 ? 1 : 0)} ${units[unit]}`;
}

async function render() {
  const container = document.querySelector<HTMLDivElement>("#downloads");
  const releaseLine = document.querySelector<HTMLParagraphElement>("#release-line");
  if (!container) return;

  let manifest: Manifest | null = null;
  try {
    // A relative path breaks when the page is opened without a trailing slash.
    const response = await fetch(`${import.meta.env.BASE_URL}download-manifest.json`);
    if (response.ok) manifest = (await response.json()) as Manifest;
  } catch {
    manifest = null;
  }

  if (!manifest || manifest.assets.length === 0) {
    const url = manifest?.releaseUrl ?? "https://github.com/SpiritUrban/web-radar/releases";
    container.innerHTML = `<a class="download primary" href="${url}" rel="noopener">Усі збірки на GitHub</a>`;
    if (releaseLine) {
      releaseLine.textContent = manifest
        ? `Готових інсталяторів для версії ${manifest.version} ще немає — вони з'являться після першого релізу.`
        : "Перелік збірок недоступний — відкрийте сторінку релізів на GitHub.";
    }
    return;
  }

  const found = CARDS.map((card) => ({
    card,
    asset: manifest!.assets.find(
      (asset) =>
        asset.platform === card.platform &&
        asset.architecture === card.architecture &&
        card.kinds.includes(asset.kind),
    ),
  })).filter((entry) => entry.asset);

  container.innerHTML = found
    .map(
      ({ card, asset }) => `
        <a class="download" href="${asset!.downloadUrl}" rel="noopener" title="${asset!.fileName}">
          <span class="platform">${card.title}</span>
          <span class="note">${card.note}</span>
          <span class="size">${formatBytes(asset!.sizeBytes)}</span>
        </a>`,
    )
    .join("");

  if (releaseLine) {
    const published = manifest.publishedAt
      ? new Date(manifest.publishedAt).toLocaleDateString("uk-UA")
      : "";
    releaseLine.innerHTML =
      `Версія <b>${manifest.version}</b>${published ? ` · ${published}` : ""} · ` +
      `<a href="${manifest.releaseUrl}" rel="noopener">усі файли релізу</a>`;
  }
}

void render();
