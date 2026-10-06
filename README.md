<div align="center">
  <img src="subclave.png" width="120" height="120" alt="Subclave" />
  <h1>Subclave</h1>
  <p><em>A local-first password manager.</em></p>
  <p>
    <img src="https://img.shields.io/badge/license-Apache--2.0-green" alt="license" />
    <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey" alt="platform" />
  </p>
</div>

Local password vaults keep secrets off vendor clouds, but moving the vault
between machines means copying a database file by hand or trusting a file-sync
service that leaves conflict copies, and filling a login still means switching
windows. Subclave is a local-first desktop password manager: the vault lives on
your machine and syncs end to end through storage you own.

## Status

v0.1.0 is the first release: an encrypted vault, sync to your own S3 bucket or WebDAV share, and a browser extension for Chromium browsers and Firefox.

## Install the browser extension

Releases after v0.1.0 attach the extension next to the installers:
`subclave-chrome.zip` and `subclave-firefox.zip`. It is not in the Chrome Web
Store or signed by Mozilla yet, so it is loaded by hand.

First, in Subclave, open Settings > Browser and turn on "Chromium browsers" or
"Firefox". That writes the native messaging manifest the extension connects
through.

### Chromium browsers (Chrome, Chromium, Edge, Brave, Vivaldi)

1. Unzip `subclave-chrome.zip` into a folder you keep. The browser loads the
   extension from that folder every time it starts.
2. Open `chrome://extensions`, turn on Developer mode and click Load unpacked.
3. Pick the unzipped folder. The extension's card shows the ID
   `fbefjngeldidlcdilbigapddeimhklpn`, the only origin Subclave's native
   messaging manifest allows. The manifest pins a `key`, so every unpacked
   copy gets this ID.
4. With Subclave running and unlocked, open the browser's Extensions menu on
   the toolbar, click Subclave (pin it to keep it on the toolbar), then Pair
   with Subclave. Click Allow in Subclave once its code matches the one in the
   popup.

To update, replace the folder's contents with the next release's
`subclave-chrome.zip` and click reload on the extension's card. The browser
does not update an unpacked extension by itself.

### Firefox

Release and Beta Firefox install only add-ons signed by Mozilla, and this one
is not signed yet:

- Release or Beta Firefox: open `about:debugging#/runtime/this-firefox`, click
  Load Temporary Add-on and pick `subclave-firefox.zip`. Firefox removes it
  when it restarts, so it has to be loaded again after every restart.
- Firefox ESR, Developer Edition or Nightly: set
  `xpinstall.signatures.required` to `false` in `about:config`, then in
  `about:addons` pick Install Add-on From File in the gear menu and choose
  `subclave-firefox.zip`. It stays installed.

Pair from the Extensions menu as in step 4 above. If the popup offers Pair
with Subclave again after a restart, pair again and revoke the old entry under
Settings > Browser.

## Fill a login

Click a login field, or the key icon in it, to list the logins saved for that
page's exact host, and pick one.

Logins for another host of the same domain (an entry for `github.com` opened
from `gist.github.com`) are not listed. The picker's last row,
`N more logins on github.com`, opens the Subclave popup, which lists every
match. Where the browser cannot open the popup from the page, the row turns
into text; then open Subclave from the browser's Extensions menu (or the
toolbar once pinned) and pick the login there.

`Ctrl+Shift+L` (`Cmd+Shift+L` on macOS) fills the current page without the
picker: one match fills right away; several open the popup, or fill the most
recently used login where the browser cannot open it.

The browser leaves the shortcut unset when another extension already holds
it. Set it at `chrome://extensions/shortcuts`, or in Firefox under
`about:addons`, gear menu, Manage Extension Shortcuts.

## Build from source

Needs Rust stable, Node 20.19+ with pnpm, and
[Tauri's prerequisites](https://tauri.app/start/prerequisites/).

```bash
pnpm install
pnpm tauri:dev     # dev build, separate data dir
pnpm tauri build   # installers
```

Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Design:
[ARCHITECTURE.md](ARCHITECTURE.md). Accepted limits:
[KNOWN-LIMITS.md](KNOWN-LIMITS.md). Security: [SECURITY.md](SECURITY.md).

## Uninstall

The Windows uninstaller removes the browser integration itself: the HKCU
native messaging keys and the manifests under
`%APPDATA%\dev.rendy.subclave\browser-hosts\`. The `.deb`, `.rpm`, AppImage and
macOS removals cannot reach per-user files, so before uninstalling there, turn
off both switches in Settings > Browser. That removes the manifest from every
browser Subclave still detects.

Per-user files an uninstall leaves behind:

- Linux and macOS: the `dev.rendy.subclave.json` manifest in each browser's
  native messaging directory, unless the switches were turned off first.
  - Linux: `~/.config/<browser>/NativeMessagingHosts/` for Chrome, Chromium,
    Edge, Brave and Vivaldi (`google-chrome`, `chromium`, `microsoft-edge`,
    `BraveSoftware/Brave-Browser`, `vivaldi`), the Snap and Flatpak copies
    under `~/snap/` and `~/.var/app/`, and for Firefox
    `~/.mozilla/native-messaging-hosts/`,
    `~/snap/firefox/common/.mozilla/native-messaging-hosts/` and
    `~/.var/app/org.mozilla.firefox/.mozilla/native-messaging-hosts/`.
  - macOS: `~/Library/Application Support/<browser>/NativeMessagingHosts/`
    (`Google/Chrome`, `Chromium`, `Microsoft Edge`,
    `BraveSoftware/Brave-Browser`, `Vivaldi`), and for Firefox
    `~/Library/Application Support/Mozilla/NativeMessagingHosts/`.
- The AppImage's `subclave-proxy` copy, inside the app data dir.
- The app data dir itself, which holds the vault: Linux
  `~/.local/share/dev.rendy.subclave/`, macOS
  `~/Library/Application Support/dev.rendy.subclave/`, Windows
  `%APPDATA%\dev.rendy.subclave\`. Keep it, or a backup, until you no longer
  need the vault.
- The logs: Linux `~/.local/share/dev.rendy.subclave/logs/`, macOS
  `~/Library/Logs/dev.rendy.subclave/`, Windows
  `%LOCALAPPDATA%\dev.rendy.subclave\logs\`.

## Credits

Derived from [Tervia](https://github.com/rendyuwu/tervia) by
[rendyuwu](https://github.com/rendyuwu), itself derived from
[TEDI](https://github.com/IlhamriSKY/TEDI) by
[IlhamriSKY](https://github.com/IlhamriSKY) and
[Terax](https://github.com/crynta/terax-ai) by
[Crynta](https://github.com/crynta), both Apache-2.0. This repo starts from the
imported Tervia v0.1.3 tree.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
