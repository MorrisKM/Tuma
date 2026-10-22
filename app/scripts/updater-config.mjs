// Writes src-tauri/tauri.updater.conf.json with the update signing key's public half, from TUMA_UPDATER_PUBKEY (the
// contents of the .pub file `tauri signer generate` writes). Release builds pass it to `tauri build --config`; the
// app updates itself only when its config has this key. The file is generated, not committed.

import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const pubkey = (process.env.TUMA_UPDATER_PUBKEY ?? "").trim();
if (!pubkey) {
  console.error("Set TUMA_UPDATER_PUBKEY to the update signing key's public half (see docs/RELEASING.md).");
  process.exit(1);
}
const out = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "tauri.updater.conf.json");
writeFileSync(out, `${JSON.stringify({ plugins: { updater: { pubkey } } }, null, 2)}\n`);
console.log(`Wrote ${out}`);
