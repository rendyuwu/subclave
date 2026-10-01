import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

// One config, driven by the environment `scripts/build.mjs` sets before each
// invocation. Three separate builds run per target (popup, background,
// content), because a shared chunk between the service worker and the content
// script is exactly what MV3 forbids and multiple IIFE inputs are not a
// supported combination.
const extDir = path.dirname(fileURLToPath(import.meta.url));

type Entry = "popup" | "background" | "content";

const entry = (process.env.EXT_ENTRY ?? "popup") as Entry;
// Absolute: `build.outDir` resolves relative to `root`, so a relative path would
// land inside `src/popup` for the popup invocation.
const outDir =
  process.env.EXT_OUT ?? path.resolve(extDir, "dist", process.env.EXT_TARGET ?? "chrome");

function outputOptions() {
  if (entry === "popup") {
    return {
      input: path.resolve(extDir, "src", "popup", "popup.html"),
      output: {
        entryFileNames: "popup.js",
        chunkFileNames: "popup-[name].js",
        assetFileNames: "popup.[ext]",
      },
    };
  }
  // `background` and `content` are one input each, so there are no chunks and
  // no `import` left for a non-module service worker.
  const input =
    entry === "content"
      ? path.resolve(extDir, "src", "content", "index.ts")
      : (process.env.EXT_ENTRY_FILE ?? path.resolve(extDir, "src", "entry.ts"));
  return {
    input,
    output: {
      format: "iife",
      entryFileNames: entry === "content" ? "content.js" : "background.js",
    },
  };
}

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // The popup entry sets root to `src/popup` so `popup.html` lands at the
  // output root; with the extension directory as root it would land at
  // `<out>/src/popup/popup.html` and `action.default_popup` would be dead.
  root: entry === "popup" ? path.resolve(extDir, "src", "popup") : extDir,
  // `scripts/build.mjs` owns the icon copy, so no build auto-copies `public/`.
  publicDir: false,
  build: {
    outDir,
    emptyOutDir: false,
    target: "es2022",
    sourcemap: false,
    rolldownOptions: outputOptions(),
  },
});
