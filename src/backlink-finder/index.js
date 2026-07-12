import { mkdir, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

import { generateSearchQueries } from "./search-queries.js";
import { collectUrls } from "./collect-urls.js";
import { checkLinks } from "./check-links.js";

export async function findBacklinks({
  domain,
  inputFile,
  outputFile,
}) {
  if (!domain) {
    throw new Error("Domain is required.");
  }

  console.log(`\nTarget domain: ${domain}`);

  const searchQueries = generateSearchQueries(domain);

  console.log("\nSearch queries:\n");

  for (const query of searchQueries) {
    console.log(`[${query.engine}] ${query.query}`);
    console.log(query.url);
    console.log();
  }

  console.log(`Reading candidate URLs from: ${inputFile}`);

  const urls = await collectUrls(inputFile);

  if (urls.length === 0) {
    console.log("\nNo URLs found.");
    console.log(`Add one URL per line to: ${inputFile}`);
    return;
  }

  console.log(`Found ${urls.length} candidate URLs.`);
  console.log("Checking pages...\n");

  const backlinks = await checkLinks({
    urls,
    targetDomain: domain,
  });

  const report = {
    targetDomain: domain,
    createdAt: new Date().toISOString(),
    candidatesChecked: urls.length,
    pagesWithBacklinks: backlinks.length,
    backlinks,
  };

  await mkdir(dirname(outputFile), {
    recursive: true,
  });

  await writeFile(
    outputFile,
    JSON.stringify(report, null, 2),
    "utf8",
  );

  console.log("\nCompleted.");
  console.log(`Pages checked: ${urls.length}`);
  console.log(`Pages with backlinks: ${backlinks.length}`);
  console.log(`Report: ${outputFile}`);

  return report;
}