# CodexSwitcher

[中文说明](README.zh-CN.md)

CodexSwitcher is a Tauri v2 desktop utility for managing multiple Codex / ChatGPT account profiles on one machine. It can import the current `~/.codex/auth.json`, save it as an encrypted local profile, probe usage, switch the global Codex auth file, and migrate all account profiles to another computer.

> This tool manages account files you already own. It does not automate login, bypass verification, or bypass platform limits.

## Screenshot

![Dashboard screenshot](docs/screenshots/dashboard.svg)

## Features

- Manage multiple account profiles with alias, enabled state, priority, cooldown, and quota rules.
- Probe Codex / ChatGPT usage through `https://chatgpt.com/backend-api/wham/usage`.
- Configure an HTTP or SOCKS proxy for probe-related requests.
- Switch the global `~/.codex/auth.json` with backup and directory locking.
- Detect running Codex processes before switching, with a force-switch option.
- Keep the current account token under Codex control to avoid refresh-token conflicts.
- One-click migration of all profiles, rules, app settings, and selected `.codex` files.
- Optional conversation migration for `sessions/`, `session_index.jsonl`, `logs_2.sqlite*`, and `state_5.sqlite*`.
- System tray support with quick actions.
- UI language can follow the system language or be set to Simplified Chinese, English, or Traditional Chinese.
- Windows, macOS, and Linux builds via GitHub Actions.

## Security Model

Local profiles are stored in the app data directory. Each profile contains an encrypted snapshot of an account `auth.json`.

On Windows, the default app data path is:

```text
C:\Users\<you>\AppData\Roaming\local.codex.account-switcher\
```

Important files:

- `store.json`: app settings, profile metadata, quota state, encrypted profile snapshots, and operation events.
- `local-profile.key`: local fallback key used only if the OS keyring cannot be used.

Migration bundles support two modes:

- Empty password: plain `.zip` bundle. This is convenient but contains sensitive auth data in readable form.
- Password set: encrypted `.zip.enc` bundle. Passwords must be at least 8 characters.

The migration bundle never includes machine-bound or sandbox files:

- `installation_id`
- `cap_sid`
- `.sandbox/`
- `.sandbox-bin/`
- `.sandbox-secrets/`
- temporary files and machine-bound logs

## What Gets Migrated

Default migration includes:

- All account profiles
- Full `auth.json` snapshot for each profile
- Alias, enabled state, priority, cooldown, quota rules, and usage state
- App settings, including Codex directory, proxy settings, and refresh settings
- `config.toml`
- `rules/`
- `memories/`

If conversation migration is enabled, it also includes:

- `sessions/`
- `session_index.jsonl`
- `logs_2.sqlite*`
- `state_5.sqlite*`

## Development

Requirements:

- Node.js LTS
- Rust stable
- Platform-specific Tauri prerequisites

Install dependencies:

```bash
npm install
```

Run the desktop app in development:

```bash
npm run tauri -- dev
```

Build the frontend:

```bash
npm run build
```

Run Rust tests:

```bash
cd src-tauri
cargo test
```

Build installers locally:

```bash
npm run tauri -- build
```

Build outputs are created under:

```text
src-tauri/target/release/bundle/
```

## GitHub Actions

This repository includes workflows for CI and release builds.

- `.github/workflows/ci.yml`
  - Runs frontend build and Rust tests on pull requests, pushes, and manual dispatch.
- `.github/workflows/release.yml`
  - Builds Windows, macOS, and Linux desktop artifacts.
  - Runs manually or when pushing tags like `codex-account-switcher-v0.1.1`.
  - On version tags, uploads installers, updater signatures, and `latest.json` to a GitHub Release.

### Auto Update Setup

CodexSwitcher uses the Tauri v2 updater with GitHub Releases:

- Update endpoint: `https://github.com/doit9816/codex-account-switcher/releases/latest/download/latest.json`
- The public updater key is stored in `src-tauri/tauri.conf.json`.
- The private updater key must stay secret and be configured in GitHub Actions secrets:
  - `TAURI_SIGNING_PRIVATE_KEY`
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` can be empty if the generated key has no password.

Generate or rotate an updater key with:

```bash
npm run tauri signer generate -- --ci -w ~/.codexswitcher-updater.key
```

When publishing a new version, update the version in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`, then push a version tag. Installed apps will find the new stable release through the in-app update checker.

Create a release:

```bash
git tag codex-account-switcher-v0.1.3
git push origin codex-account-switcher-v0.1.3
```

If release upload fails with a token permission error, set the repository's GitHub Actions workflow permissions to **Read and write permissions**.

## Publishing Only This Tool

This folder is designed to be published as its own GitHub repository root. Do not publish the parent CMS repository if you only want to share this tool.

One direct flow:

```bash
cd tools/codex-account-switcher
git init
git add .
git commit -m "Initial CodexSwitcher release"
git branch -M main
git remote add origin git@github.com:<owner>/<repo>.git
git push -u origin main
```

Or use the included PowerShell helper from the parent workspace:

```powershell
tools/codex-account-switcher/scripts/publish-to-github.ps1 `
  -RemoteUrl git@github.com:<owner>/<repo>.git
```

Then push a version tag to trigger release builds:

```bash
git tag codex-account-switcher-v0.1.3
git push origin codex-account-switcher-v0.1.3
```

## Notes

- macOS and Windows release artifacts generated by GitHub Actions are unsigned by default.
- Unsigned macOS apps may be blocked by Gatekeeper with an "is damaged and can't be opened" message. This usually means the app is not Apple Developer signed and notarized, not that the download is corrupted.
- Unsigned Windows installers may trigger SmartScreen until you add code signing.
- No open-source license is included yet. Add a `LICENSE` file before publishing publicly if you want to define reuse terms.

### macOS "damaged" warning

The current release artifacts are not Apple-notarized. If macOS says the app is damaged, remove the quarantine flag before opening the DMG:

```bash
xattr -dr com.apple.quarantine ~/Downloads/CodexSwitcher_0.1.3_aarch64.dmg
open ~/Downloads/CodexSwitcher_0.1.3_aarch64.dmg
```

If you already copied the app to Applications, run:

```bash
xattr -dr com.apple.quarantine "/Applications/CodexSwitcher.app"
open "/Applications/CodexSwitcher.app"
```

Once Apple Developer signing and notarization are configured, this workaround is no longer needed.
