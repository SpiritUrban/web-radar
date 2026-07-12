import { findBacklinks } from "../backlink-finder/index.js";

const TARGET_DOMAIN = "my-transfer.com.ua";

try {
  await findBacklinks({
    domain: TARGET_DOMAIN,
    inputFile: "data/input/urls.txt",
    outputFile: "data/output/backlinks.json",
  });
} catch (error) {
  console.error("\nFatal error:");
  console.error(error);
  process.exitCode = 1;
}