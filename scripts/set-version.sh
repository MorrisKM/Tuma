#!/usr/bin/env bash
# Sets the release version everywhere it is written: the Cargo workspace (which the app, the CLI and Tauri's bundles
# read), app/package.json, Cargo.lock, and the AppStream release list. Usage: scripts/set-version.sh 1.2.3
set -euo pipefail

version="${1:?usage: scripts/set-version.sh X.Y.Z}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "not a version: $version" >&2
  exit 2
fi
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

sed -i.bak -E "0,/^version = \".*\"/s//version = \"$version\"/" Cargo.toml && rm Cargo.toml.bak
node -e '
  const fs = require("fs");
  const p = "app/package.json";
  const pkg = JSON.parse(fs.readFileSync(p, "utf8"));
  pkg.version = process.argv[1];
  fs.writeFileSync(p, JSON.stringify(pkg, null, 2) + "\n");
' "$version"
cargo update --workspace --quiet

metainfo=packaging/linux/dev.tuma.app.metainfo.xml
if ! grep -q "release version=\"$version\"" "$metainfo"; then
  today="$(date -u +%Y-%m-%d)"
  sed -i.bak "s|<releases>|<releases>\n    <release version=\"$version\" date=\"$today\"/>|" "$metainfo" && rm "$metainfo.bak"
fi
echo "Version $version. Add its notes to CHANGELOG.md, commit, then tag v$version."
