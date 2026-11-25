# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Documentation
- README screenshots (main window, search, Quick Tidy, settings) rendered from synthetic demo data.

## [0.1.0] - 2026-11-25

First release.

### Added
- Find screenshots across the Mac (Spotlight + name patterns), in folders of third-party capture
  apps (auto-detected), and in the Photos library via PhotoKit.
- Live updates: watches screenshot folders and analyzes new screenshots as they appear.
- On-device analysis: text recognition (Korean/English), barcodes/QR, Vision image classification,
  and picture descriptions with Apple's on-device foundation model (macOS 27+).
  Analysis runs once, is cached, and resumes after an interruption; a progress window covers the
  first run.
- Smart categories: *Safe to Delete* and *Personal Info*, with tag filters.
- Spotlight-style search (`⌘F`) by text, picture content, app, device or tag, in four languages.
- Similar screenshots grouped by Vision feature prints and capture time; side-by-side compare;
  **Quick Tidy** keeps the best shot per group.
- Organize into dated folders; Photos items are exported and then removed from Photos.
- Device detection (iPhone/iPad/Mac/Apple Watch) and app detection (Instagram, YouTube,
  web browser, messenger, code editor, …).
- Manual category corrections and *Keep* marks.
- Liquid Glass design (native sidebar on macOS 26+), light/dark theme, responsive toolbar,
  menu-bar icon and menu; the app stays in the menu bar when the window is closed.
- Settings: theme, language (English, 한국어, 日本語, Español), organize folder and layout,
  macOS screenshot location, search scope, analysis options, cache management.

[Unreleased]: https://github.com/hoyakdh/clear-screenshot/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hoyakdh/clear-screenshot/releases/tag/v0.1.0
