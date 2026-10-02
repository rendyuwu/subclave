import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Zips the two store variants into `dist/`. CI uploads `extension/dist/*.zip`.
const extDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const webExt = process.platform === "win32" ? "web-ext.cmd" : "web-ext";

for (const target of ["chrome", "firefox"]) {
  const run = spawnSync(
    webExt,
    [
      "build",
      "--source-dir",
      path.join(extDir, "dist", target),
      "--artifacts-dir",
      path.join(extDir, "dist"),
      "--filename",
      `subclave-${target}.zip`,
      "--overwrite-dest",
    ],
    { stdio: "inherit" },
  );
  if (run.status !== 0) process.exit(run.status ?? 1);
}

console.log("zip: dist/subclave-chrome.zip and dist/subclave-firefox.zip");
