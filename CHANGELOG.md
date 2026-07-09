# Changelog

All notable changes to Tauridium are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project adheres to [Semantic Versioning](https://semver.org/).

> Release notes **must be written in English**. The section of the tagged
> version is picked up **automatically** as the GitHub Release notes (see
> `.github/workflows/release.yml`), so fill in the section **before** pushing
> the `vX.Y.Z` tag. Versions ≤ 0.1.8 predate this changelog — see the git
> history and the [GitHub releases](https://github.com/Gizmo091/Tauridium/releases).

## [Unreleased]

## [0.1.9] - 2026-07-09
### Fixed
- **Links in conversations are clickable again.** `target="_blank"` links and
  `window.open` calls no longer responded to clicks on macOS. They now open in
  your default browser, while sign-in popups (sized OAuth windows) open in an
  in-app window so the session is preserved and the service webview stays in
  place.
