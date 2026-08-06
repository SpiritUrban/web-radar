#!/usr/bin/env node
/**
 * Fail the release before anything is built if the version strings disagree
 * with each other or with the tag being released.
 *
 * A mismatch between Cargo.lock and package.json makes the Tauri CLI stop with
 * "Found version mismatched Tauri packages" four minutes into a build; catching
 * it here costs two seconds.
 */
import { VERSION_PATTERN, collectVersions } from "./version-files.mjs";

for (const stream of [process.stdout, process.stderr]) {
  stream.setDefaultEncoding?.("utf8");
}

const found = collectVersions();
if (found.length === 0) {
  console.error("::error title=Version check::no version strings were found at all — the parser is broken, not the repo");
  process.exit(1);
}

let failed = false;
const missing = found.filter((entry) => !entry.value);
if (missing.length > 0) {
  console.error(`::error title=Version check::could not read a version from: ${missing.map((e) => e.label).join(", ")}`);
  failed = true;
}

const versions = [...new Set(found.filter((e) => e.value).map((e) => e.value))];
if (versions.length > 1) {
  console.error("::error title=Version check::files disagree about the version");
  for (const entry of found) console.error(`  ${entry.label}: ${entry.value ?? "—"}`);
  failed = true;
}

const ref = process.env.GITHUB_REF_NAME ?? "";
if (ref.startsWith("v")) {
  const tagVersion = ref.slice(1);
  if (!VERSION_PATTERN.test(tagVersion)) {
    console.error(`::error title=Version check::tag "${ref}" is not vX.Y.Z`);
    failed = true;
  } else if (versions.length === 1 && versions[0] !== tagVersion) {
    console.error(
      `::error title=Version check::tag ${ref} does not match the repository version ${versions[0]}` +
        ` — run: node scripts/sync-version.mjs ${tagVersion}`,
    );
    failed = true;
  }
}

if (failed) process.exit(1);

console.log(`Version ${versions[0]} is consistent across ${found.length} places${ref ? ` and matches ${ref}` : ""}.`);
for (const entry of found) console.log(`  ${entry.label}: ${entry.value}`);
