#!/usr/bin/env node
/**
 * Set the release version everywhere at once.
 *
 *   node scripts/sync-version.mjs 0.4.0
 *
 * Then commit, then tag. `check-version.mjs` (run by CI before any build)
 * verifies that every file agrees with the tag.
 */
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { ROOT, TARGETS, VERSION_PATTERN, collectVersions, read } from "./version-files.mjs";

// Windows runners print through cp1252; captured output can contain anything.
for (const stream of [process.stdout, process.stderr]) {
  stream.setDefaultEncoding?.("utf8");
}

const version = (process.argv[2] ?? "").replace(/^v/, "");
if (!VERSION_PATTERN.test(version)) {
  console.error(`usage: node scripts/sync-version.mjs <x.y.z>   (got "${process.argv[2] ?? ""}")`);
  process.exit(1);
}

for (const target of TARGETS) {
  const before = read(target.file);
  const after = target.set(before, version);
  if (before === after) {
    console.log(`  = ${target.file} (already ${version})`);
    continue;
  }
  writeFileSync(resolve(ROOT, target.file), after);
  console.log(`  ✓ ${target.file}`);
}

const mismatched = collectVersions().filter((entry) => entry.value !== version);
if (mismatched.length > 0) {
  // A rewrite that silently matched nothing is the failure mode this catches.
  console.error("\nThese files still disagree after the rewrite:");
  for (const entry of mismatched) console.error(`  ${entry.label}: ${entry.value ?? "not found"}`);
  process.exit(1);
}

console.log(`\nAll files are at ${version}. Next: commit, then tag v${version}.`);
