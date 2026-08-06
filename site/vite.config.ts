import { defineConfig } from "vite";

/**
 * The site lives in a subdirectory on GitHub Pages
 * (https://spiriturban.github.io/web-radar/), so every asset URL has to carry
 * that prefix. A leading `/` would point at the domain root and 404.
 */
export default defineConfig({
  root: __dirname,
  base: process.env.GITHUB_PAGES ? "/web-radar/" : "/",
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
