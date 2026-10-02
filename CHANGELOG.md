# Changelog

The latest release only. Every earlier version:
[GitHub Releases](https://github.com/rendyuwu/subclave/releases). Versions:
[SemVer](https://semver.org/); before `1.0` a minor bump may break things.

## [0.1.0] - 02-10-2026

### Added

- Desktop app for macOS, Linux and Windows, derived from Tervia v0.1.3, with signed in-app updates.
- Encrypted vault: one file sealed with AES-256-GCM under an Argon2id key, a `.bak` snapshot, master password change, idle and minimize auto-lock, and a tray icon.
- Entries and nested groups with tags, favourites, expiry dates, search, Trash and per-entry version history.
- Copy with clipboard auto-clear, TOTP codes, and a password generator with a strength meter.
- End-to-end encrypted sync to your own S3-compatible bucket or WebDAV share, with a merge that keeps both sides of a conflict.
- Browser extension for Chromium browsers and Firefox: pairing, an icon and picker inside login fields, popup fill, the fill command, and "Generate for this site".
- CSV import from KeePassXC, Bitwarden, Chrome and Firefox, CSV export, and encrypted `.subclave-backup` files.
