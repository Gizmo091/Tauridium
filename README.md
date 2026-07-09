<p align="center">
  <img src="src-tauri/icons/icon.png" width="128" alt="Tauridium icon" />
</p>

<h1 align="center">Tauridium</h1>

<p align="center">
  <a href="https://github.com/Gizmo091/Tauridium/releases/latest"><img src="https://img.shields.io/github/v/release/Gizmo091/Tauridium?sort=semver" alt="Latest release" /></a>
  <a href="https://github.com/Gizmo091/Tauridium/releases"><img src="https://img.shields.io/github/downloads/Gizmo091/Tauridium/total?label=downloads%20total" alt="Total downloads (all releases)" /></a>
  <a href="https://github.com/Gizmo091/Tauridium/releases/latest"><img src="https://img.shields.io/github/downloads/Gizmo091/Tauridium/latest/total?label=downloads%20latest" alt="Downloads (latest release)" /></a>
  <img src="https://img.shields.io/badge/Maintained%3F-yes-green.svg" alt="Maintained: yes" />
  <img src="https://img.shields.io/badge/maintainer-Mathieu%20Vedie-blue" alt="Maintainer: Mathieu Vedie" />
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT" /></a>
  <img src="https://img.shields.io/badge/contributions-welcome-brightgreen.svg" alt="Contributions welcome" />
  <a href="https://github.com/Gizmo091/Tauridium/commits"><img src="https://badgen.net/github/last-commit/Gizmo091/Tauridium" alt="Latest commit" /></a>
</p>

<p align="center">
  <a href="https://www.buymeacoffee.com/mathieuvedie"><img src="https://www.buymeacoffee.com/assets/img/custom_images/orange_img.png" alt="Buy Me A Coffee" height="32" /></a>
</p>

A lightweight desktop client for [Ferdium](https://ferdium.org), built with
**Tauri v2** (Rust + native WebView) instead of Electron — while staying fully
connected to your Ferdium **server** (same account, synced services & workspaces).

> ⚡️ **This project is vibe-coded.** It was built end-to-end in a
> pair-programming session with an AI assistant (Claude / Claude Code). The
> architecture, code, icon and CI/CD were shaped conversationally rather than
> from a formal spec — treat it accordingly. 🤖

The name is a nod to the lineage **Franz → Ferdi → Ferdium**, with the `-ium`
suffix kept and **Tauri** baked in.

## Why

Ferdium is great, but Electron makes it heavy. Tauridium renders each service in
its own **isolated native WebView** (per-service persistent sessions), talks to
the **Ferdium server REST API** for auth / services / workspaces, and stays lean.

## Features

- Sign in to your Ferdium server — account, services and workspaces stay synced
- Each service in an **isolated, persistent session** (native WebView)
- **Native notifications** + dock unread badges
- **Close-to-tray**, run in background, launch at login
- **Per-service settings** (name, custom URL, team, notifications, mute, badges,
  hibernation, dark mode, favicon, proxy, custom user agent…) — synced with Ferdium
- **App settings in tabs**: General / Services / Appearance / Privacy / Advanced
- **Sidebar customization** aligned with Ferdium (icon size, services location,
  grayscale + dim level, width)
- Theme (system / dark / light) + accent color (Tauri yellow by default)

## Tech stack

- **Tauri v2** (Rust) — multi-webview, tray, native notifications
- **Svelte 5** + TypeScript — the shell UI
- **reqwest** (rustls) — server calls from Rust (no CORS, token kept out of JS)
- Vendored + patched **wry** — unfreezes `window.ipc` so Electron-style recipe
  IPC works (e.g. Synology Chat)

## Develop

Requirements: Rust (stable), Node 20+, and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```bash
npm install
cargo tauri dev        # or: npm run tauri dev
```

### Tests

```bash
npm test                                          # frontend (vitest)
cargo test --manifest-path src-tauri/Cargo.toml   # Rust
```

## Build

```bash
cargo tauri build              # release bundle for your platform
cargo tauri build --debug      # faster debug bundle
```

On macOS, signing locally with your own identity avoids repeated Keychain
prompts (WebView session stores are Keychain-encrypted; a stable signature makes
"Always Allow" stick):

```bash
APPLE_SIGNING_IDENTITY="Apple Development: …" cargo tauri build --debug
```

## Releases

Pushing a `v*` tag triggers GitHub Actions, which runs the tests, builds for
**macOS** (universal), **Linux** (x86_64 / ARM64) and **Windows** (x86_64 /
ARM64), then — once every build passes — publishes a GitHub Release with the
bundles attached.

Release notes come from [`CHANGELOG.md`](CHANGELOG.md): the workflow extracts the
section matching the tagged version and uses it as the GitHub Release body. So,
before tagging:

1. Move the relevant `## [Unreleased]` entries into a new `## [X.Y.Z] - DATE`
   section (**write them in English** — this is the project convention).
2. Bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and
   `src-tauri/Cargo.lock`, then commit.
3. Tag and push:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

If no matching `CHANGELOG.md` section exists, the workflow falls back to generic
notes (and logs a warning).

Continuous integration (`cargo fmt` · clippy · Rust tests · svelte-check ·
vitest · frontend build) runs on every push and pull request.

## Install

Grab the asset for your platform from the
[latest release](../../releases/latest).

**macOS** builds are **unsigned** (no paid Apple Developer account), so Gatekeeper
blocks them on first launch (*"Tauridium can't be opened…"*). Open the `.dmg`,
drag Tauridium to Applications, then either:

- **macOS ≤ 14**: right-click the app → **Open** → confirm; or
- **macOS 15+**: try to open it, then **System Settings → Privacy & Security →
  Open Anyway**; or
- run once in Terminal: `xattr -cr /Applications/Tauridium.app`

**Linux**: `.deb` / `.rpm` / `.AppImage` (x86_64 and ARM64).
**Windows**: `.msi` or `-setup.exe` (x64 and ARM64).

## Known limitations

- **Passkeys / biometric sign-in (Touch ID, security keys) don't work.** This is
  a WebKit limitation: WebAuthn is disabled in an embedded `WKWebView` unless the
  app holds Apple's restricted *Web Browser* entitlement (granted only to real
  browsers). It affects every service, not just Google. **Workaround:** on the
  login screen pick "try another way" and use a **password + an authenticator
  code (TOTP) or a phone prompt** instead of a passkey. See
  [tauri-apps/tauri#7926](https://github.com/tauri-apps/tauri/issues/7926).

## Status & caveats

- Vibe-coded personal project — expect rough edges.
- Primary target is **macOS**; other platforms build in CI but are less
  battle-tested.
- macOS builds are **unsigned** (see [Install](#install)) — proper Developer ID
  signing + notarization needs a paid Apple Developer account, wired in CI and
  ready to activate via secrets.
- Not affiliated with Ferdium.
