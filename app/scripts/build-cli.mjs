// Builds the `tuma` command for the target being bundled and puts it where Tauri's externalBin expects it
// (src-tauri/binaries/tuma-<target triple>), so the app packages ship the CLI too. Run by tauri.release.conf.json.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const app = join(dirname(fileURLToPath(import.meta.url)), "..");
const root = join(app, "..");
const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (.+)$/m)[1];
// Tauri sets this to the --target being built; a plain build is for the host.
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || host;
const exe = triple.includes("windows") ? ".exe" : "";

execFileSync("cargo", ["build", "--release", "--locked", "-p", "tuma-cli", "--target", triple], {
  cwd: root,
  stdio: "inherit",
});
const out = join(app, "src-tauri", "binaries");
mkdirSync(out, { recursive: true });
copyFileSync(join(root, "target", triple, "release", `tuma${exe}`), join(out, `tuma-${triple}${exe}`));
console.log(`Bundling the tuma CLI for ${triple}`);
