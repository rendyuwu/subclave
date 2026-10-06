# Changelog

The latest release only. Every earlier version:
[GitHub Releases](https://github.com/rendyuwu/subclave/releases). Versions:
[SemVer](https://semver.org/); before `1.0` a minor bump may break things.

## [0.2.0] - 06-10-2026

### Changed

- The inline picker's "more logins" footer is now a row that opens the Subclave popup, which lists the logins saved for other hosts of the domain. Where the browser cannot open the popup from the page, the footer names the Extensions menu and the fill shortcut instead.

### Fixed

- The "Join a synced vault" screen scrolls as a whole and its form widens with the window, up to the Settings content width. No horizontal scrollbar appears.
- Join vault no longer stays disabled on a blank S3 Region, which Settings > Sync already saves. While the button is disabled, a list under it names every field that is still empty or invalid.
