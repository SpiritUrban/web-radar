import { readFile } from "node:fs/promises";

export async function collectUrls(inputFile) {
  let content;

  try {
    content = await readFile(inputFile, "utf8");
  } catch (error) {
    if (error.code === "ENOENT") {
      throw new Error(
        `Input file does not exist: ${inputFile}`,
      );
    }

    throw error;
  }

  const urls = content
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .filter((line) => !line.startsWith("#"))
    .filter(isHttpUrl);

  return [...new Set(urls)];
}

function isHttpUrl(value) {
  try {
    const url = new URL(value);

    return (
      url.protocol === "http:" ||
      url.protocol === "https:"
    );
  } catch {
    console.warn(`Invalid URL skipped: ${value}`);
    return false;
  }
}