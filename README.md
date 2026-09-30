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

Early development. The app is an application shell; the vault, sync and browser extension are not built yet.

## Build from source

Needs Rust stable, Node 20.19+ with pnpm, and
[Tauri's prerequisites](https://tauri.app/start/prerequisites/).

```bash
pnpm install
pnpm tauri:dev     # dev build, separate data dir
pnpm tauri build   # installers
```

Contributing: [CONTRIBUTING.md](CONTRIBUTING.md).

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
