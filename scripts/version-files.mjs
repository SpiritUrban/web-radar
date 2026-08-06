/**
 * Where the version lives. One list, used by both sync-version and
 * check-version, so the two can never disagree about what to look at.
 */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** Crates whose version line in Cargo.lock must move with the release. */
export const WORKSPACE_CRATES = ["web-radar", "web-radar-core", "web-radar-desktop"];

export const VERSION_PATTERN = /^\d+\.\d+\.\d+$/;

export function read(relativePath) {
  return readFileSync(resolve(ROOT, relativePath), "utf8");
}

/** Every place a version is written, with how to read and rewrite it. */
export const TARGETS = [
  {
    file: "package.json",
    get: (text) => JSON.parse(text).version,
    set: (text, version) => text.replace(/("version"\s*:\s*)"[^"]+"/, `$1"${version}"`),
  },
  {
    file: "src-tauri/tauri.conf.json",
    get: (text) => JSON.parse(text).version,
    set: (text, version) => text.replace(/("version"\s*:\s*)"[^"]+"/, `$1"${version}"`),
  },
  {
    file: "Cargo.toml",
    // [workspace.package] version — the single source for all three crates.
    get: (text) => text.match(/\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/s)?.[1],
    set: (text, version) =>
      text.replace(
        /(\[workspace\.package\][^[]*?version\s*=\s*)"[^"]+"/s,
        `$1"${version}"`,
      ),
  },
  {
    // Rule 1 of STAGE2_BRIEF: Cargo.lock is committed, so it also has to move —
    // by regex, without invoking cargo, so the check needs no network.
    file: "Cargo.lock",
    get: (text) => {
      for (const crate of WORKSPACE_CRATES) {
        const found = text.match(
          new RegExp(`name = "${crate}"\\nversion = "([^"]+)"`),
        )?.[1];
        if (found) return found;
      }
      return undefined;
    },
    set: (text, version) => {
      let out = text;
      for (const crate of WORKSPACE_CRATES) {
        out = out.replace(
          new RegExp(`(name = "${crate}"\\nversion = )"[^"]+"`),
          `$1"${version}"`,
        );
      }
      return out;
    },
    /** Every workspace crate must agree, not just the first one found. */
    all: (text) =>
      WORKSPACE_CRATES.map((crate) => ({
        label: `Cargo.lock (${crate})`,
        value: text.match(new RegExp(`name = "${crate}"\\nversion = "([^"]+)"`))?.[1],
      })).filter((entry) => entry.value !== undefined),
  },
];

/** `[{ label, value }]` for every version string in the repository. */
export function collectVersions() {
  const found = [];
  for (const target of TARGETS) {
    const text = read(target.file);
    if (target.all) {
      found.push(...target.all(text));
    } else {
      found.push({ label: target.file, value: target.get(text) });
    }
  }
  return found;
}
