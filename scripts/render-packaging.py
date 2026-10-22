#!/usr/bin/env python3
"""Fills the packaging templates (packaging/**/*.in) with a release's version, download URLs and SHA-256 checksums.

    scripts/render-packaging.py --version 1.0.0 --assets <folder with the release files> --out <folder>

The release workflow runs it on the files it built; the output is attached to the draft release, ready to submit to
the Homebrew tap, winget and Flathub (docs/RELEASING.md). It fails if a file a template needs is missing.
"""

import argparse
import datetime
import hashlib
import re
import sys
from pathlib import Path

REPO = "https://github.com/MorrisKM/Tuma"

# Template keys and the release file each one stands for.
ASSETS = {
    "cli-macos-arm": r"tuma-{v}-aarch64-apple-darwin\.tar\.gz",
    "cli-macos-intel": r"tuma-{v}-x86_64-apple-darwin\.tar\.gz",
    "cli-linux-arm": r"tuma-{v}-aarch64-unknown-linux-gnu\.tar\.gz",
    "cli-linux-intel": r"tuma-{v}-x86_64-unknown-linux-gnu\.tar\.gz",
    "dmg-arm": r"Tuma_{v}_aarch64\.dmg",
    "dmg-intel": r"Tuma_{v}_x64\.dmg",
    "nsis-x64": r"Tuma_{v}_x64-setup\.exe",
    "deb-amd64": r"Tuma_{v}_amd64\.deb",
    "deb-arm64": r"Tuma_{v}_arm64\.deb",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--assets", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    files = {p.name: p for p in args.assets.iterdir() if p.is_file()}
    found = {}
    for key, pattern in ASSETS.items():
        regex = re.compile(pattern.format(v=re.escape(args.version)) + "$")
        match = next((name for name in files if regex.match(name)), None)
        if match:
            found[key] = match

    root = Path(__file__).resolve().parent.parent / "packaging"
    missing = set()

    def fill(text: str) -> str:
        def replace(m: re.Match) -> str:
            kind, key = m.group(1), m.group(2)
            if key not in found:
                missing.add(key)
                return m.group(0)
            name = found[key]
            if kind == "URL":
                return f"{REPO}/releases/download/v{args.version}/{name}"
            digest = sha256(files[name])
            return digest.upper() if kind == "SHA256UPPER" else digest

        text = text.replace("@VERSION@", args.version)
        text = text.replace("@DATE@", datetime.date.today().isoformat())
        return re.sub(r"@(URL|SHA256|SHA256UPPER):([a-z0-9-]+)@", replace, text)

    for template in sorted(root.rglob("*.in")):
        rel = template.relative_to(root)
        out = args.out / rel.parent / rel.name[: -len(".in")]
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(fill(template.read_text()))
    # The Flathub repository needs the desktop entry and metainfo next to the manifest.
    for extra in ("dev.tuma.app.metainfo.xml", "dev.tuma.app.desktop"):
        (args.out / "flatpak" / extra).write_text((root / "linux" / extra).read_text())

    if missing:
        print(f"missing release files for: {', '.join(sorted(missing))}", file=sys.stderr)
        return 1
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
