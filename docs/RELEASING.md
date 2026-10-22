# Releasing Tuma

A release is a version tag. The [release workflow](../.github/workflows/release.yml) builds everything into a **draft**
GitHub release; nothing is public until you publish the draft. The packaging manifests it attaches are then submitted
to Homebrew, winget and Flathub by hand.

## One-time setup

These need your accounts, so they are not automated. Secrets and variables go in the repository's
**Settings → Secrets and variables → Actions**.

### Update signing key (required)

The app installs only updates signed with this key, and checks them against its public half, which is compiled into
every release build.

```bash
cd app
pnpm tauri signer generate -w ~/.tauri/tuma.key     # choose a password
```

- Secret `TAURI_SIGNING_PRIVATE_KEY`: the contents of `~/.tauri/tuma.key`
- Secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password
- Variable (not secret) `TUMA_UPDATER_PUBKEY`: the contents of `~/.tauri/tuma.key.pub`. The release workflow puts it
  in the app's config; builds without it don't update themselves.

Back up the key and password somewhere safe. **If they are lost, installed copies of Tuma can never update again**;
users would have to reinstall by hand. The workflow refuses to build a release without them.

### macOS signing and notarization (strongly recommended)

Without these the app is unsigned: macOS warns that it is from an unidentified developer and users have to approve it
in System Settings, and Homebrew is phasing out unsigned casks. You need the Apple Developer Program.

- `APPLE_CERTIFICATE`: a *Developer ID Application* certificate exported as .p12, base64-encoded
  (`base64 -i cert.p12 | pbcopy`)
- `APPLE_CERTIFICATE_PASSWORD`: the .p12's password
- `APPLE_SIGNING_IDENTITY`: such as `Developer ID Application: Your Name (TEAMID)`
- `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password from appleid.apple.com) and `APPLE_TEAM_ID`, for
  notarization

### Windows signing (recommended)

Without it Windows SmartScreen warns before the installer runs, until the download builds a reputation.

- `WINDOWS_CERTIFICATE`: a code signing certificate (.pfx), base64-encoded
- `WINDOWS_CERTIFICATE_PASSWORD`

### Distribution channels

- **Homebrew:** create the tap repository `MorrisKM/homebrew-tuma` with `Formula/` and `Casks/` folders. Users then
  run `brew install morriskm/tuma/tuma` (the CLI) or `brew install --cask morriskm/tuma/tuma-app` (the app and CLI).
- **winget:** fork [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs). Each release is a pull request
  adding `manifests/m/MorrisKM/Tuma/<version>/`.
- **Flathub:** follow [the submission guide](https://docs.flathub.org/docs/for-app-authors/submission). Two decisions
  first:
  - **The app ID.** The manifest uses `dev.tuma.app`, the app's identifier, but Flathub only accepts an ID whose
    domain you can prove you control (here, `tuma.dev`). Without that domain, use `io.github.MorrisKM.Tuma`. Changing
    the identifier also changes where the app keeps its data, so decide before 1.0 is published.
  - **Building from source.** The manifest repackages the release's .deb, which Flathub accepts for some apps but
    reviewers may ask for a build from source (cargo and pnpm offline sources made with flatpak-builder-tools).

## Each release

1. Set the version and write the changelog:

   ```bash
   scripts/set-version.sh 1.2.3
   # Add a "## 1.2.3" section to CHANGELOG.md (the release notes come from it).
   ```

2. Commit, merge to `main` through a pull request so CI runs, then tag that commit and push the tag:

   ```bash
   git tag v1.2.3
   git push origin v1.2.3
   ```

3. Wait for the **Release** workflow. It checks the tag matches the version, then builds:
   - The app: Linux x64 and arm64 (.deb, .rpm, .AppImage), macOS Apple Silicon and Intel (.dmg, and .app.tar.gz for
     updates), Windows x64 (NSIS .exe and .msi). Every package includes the `tuma` command.
   - `latest.json`, which installed apps check for updates.
   - The CLI alone for five targets, as `tuma-<version>-<target>.tar.gz` (or .zip on Windows).
   - `SHA256SUMS`, and `packaging-manifests.zip` with the filled-in Homebrew, winget and Flatpak files.

4. Check the draft: install a package or two, run `tuma --version`, open the app.

5. **Publish the draft.** Installed apps see the update from then on (the update URL is the latest *published*
   release's `latest.json`).

6. Submit the manifests from `packaging-manifests.zip`:
   - Homebrew: copy `homebrew/tuma.rb` to the tap's `Formula/` and `homebrew/tuma-app.rb` to `Casks/`; commit and
     push.
   - winget: copy the three `winget/` files to `manifests/m/MorrisKM/Tuma/<version>/` in your fork and open a pull
     request (`winget validate --manifest <folder>` first, on Windows).
   - Flathub: update the manifest in the app's Flathub repository with the new `flatpak/dev.tuma.app.yml` and open a
     pull request.

If the workflow fails part way, fix the problem and run it again from the Actions tab with the tag
(**Run workflow**): it adds to the existing draft.

## Building packages locally

```bash
cd app
pnpm release        # tauri build with the release config: the CLI sidecar and update artifacts
```

It needs `TAURI_SIGNING_PRIVATE_KEY` and the matching `TUMA_UPDATER_PUBKEY` (any key pair works for a local test;
generate one as above), and builds the formats of the machine it runs on into `target/release/bundle/`.
