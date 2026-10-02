# SUBCLAVE.md

Agent memory and contributor reference for Subclave. Build and PR rules:
[CONTRIBUTING.md](CONTRIBUTING.md).

## Project

|                 |                                                                         |
| --------------- | ----------------------------------------------------------------------- |
| Version         | 0.1.0                                                                   |
| Repo            | `github.com/rendyuwu/subclave`                                          |
| Stack           | Tauri 2 + Rust with React 19 + TS, Tailwind v4 and shadcn/ui            |
| Bundle id       | `dev.rendy.subclave` (dev profile: `dev.rendy.subclave.dev`)            |
| Crates/binaries | `subclave` (lib `subclave_lib`, GUI binary `SubclaveApp`)               |
| Derived from    | Tervia, `rendyuwu/tervia@5c5f496f` (tag `tervia-baseline` in this repo) |
| Package manager | pnpm                                                                    |

## Conventions

- **Icons**: `lucide-react`, imported by name. Brand marks:
  `src/components/BrandIcon.tsx` (`github`).
- **Styling**: Tailwind v4 (`@theme` blocks in `src/styles/globals.css` and
  `src/styles/shadcn-tailwind.css`, no `tailwind.config.*`); `cn()` from
  `@/lib/utils`. shadcn/ui components are generated, not hand-edited.
- **Imports**: always `@/...`, never a relative path across modules
  (`scripts/check-imports.mjs`).
- **Types**: no `any` in TypeScript.
- **Vault secrets**: the vault UI never puts a password, a TOTP URI or a hidden
  custom value in the store, and rarely in component state. A secret is read
  back through `vault_entry_reveal` or `clip_copy_field` and lives only in the
  component showing it: a reveal (through `SecretField`) clears on blur, after
  30 s, and when the selected entry changes, and the entry editor holds the TOTP
  URI only while its dialog is open.
- **Window styling**: macOS gets native traffic lights via an Overlay title
  bar; Linux and Windows are borderless with React `WindowControls`. Windows
  adds `apply_windows_frame_fixes` (main window only, maximize-clamp and
  minimize), `disable_windows_corner_rounding`, and
  `disable_browser_accelerator_keys` so WebView2 does not eat Ctrl+W / Ctrl+R.
- **Docs and prose**: no em-dashes (commas, colons, or parentheses instead).
  No emoji in docs, code, or commits.
- **Cite a symbol, not a line**: a line number goes stale the moment another
  commit touches that file, and a path no clone can open is just as useless.
  Comments cite only what a reader holding the clone can reach: a file in the
  tree, or a symbol. `scripts/citation-format-verify.ts` fails on a `file:line`
  or a bare `:line` inside a comment in `src/`, `src-tauri/src/`, or `scripts/`,
  and on a backticked path that resolves to no file in the tree. One carve-out:
  a comment may name a file that is gone when the deletion itself is the
  sentence's subject, in the past tense ("once X was deleted, Y became
  unreachable").
- **Accepted limits**: an accepted state goes in `KNOWN-LIMITS.md` as three
  parts: what is accepted, where (file and symbol), and what would change it.
- **Commands**: every `#[tauri::command]` is `pub async fn` and does its
  blocking work on the blocking pool, never on the UI thread: vault, import and
  backup commands through `run_blocking`
  (`src-tauri/src/modules/vault/events.rs`), sync commands through `blocking`
  (`src-tauri/src/modules/sync/engine/commands.rs`), the rest through
  `tauri::async_runtime::spawn_blocking`. The `no_new_sync_tauri_commands`
  test in `src-tauri/src/commands.rs` fails on a synchronous one.

## Area rules

### Modal gate (`src/modules/shortcuts/lib/modalRegistry.ts`)

- No catalogued shortcut fires while a `Dialog`/`AlertDialog` is open. The
  registry is a stack; only the command palette's own chord is exempt, and only
  while the palette is topmost.

### Extension (`extension/`)

- Three entry points, built one Vite invocation each (`scripts/build.mjs`):
  `src/popup/popup.html` -> `popup.{html,js,css}`, `src/entry.ts` ->
  `background.js`, `src/content/index.ts` -> `content.js`. A shared chunk
  between the service worker and the content script is what MV3 forbids, so
  the three are never one multi-entry build; the two script entries are IIFE.
- One manifest per target, copied to `manifest.json`: `manifest.chrome.json`
  (service worker, pinned `key`) and `manifest.firefox.json` (background
  scripts, `browser_specific_settings`, no `key`). `dist/<target>` is generated
  and gitignored.
- `chrome.storage` is read and written only in the service worker
  (`src/background.ts`). The popup and the content script talk to it over
  `chrome.runtime` messages (`src/lib/messages.ts`).
- The content script never computes a URL match. Matching, credential release
  and every write happen in Rust
  (`src-tauri/src/modules/browser/matching.rs`,
  `src-tauri/src/modules/browser/actions.rs`); the extension's job is UI,
  transport and DOM fill.
- `src/lib/protocol.ts` and `src/lib/auth.ts` mirror
  `src-tauri/src/modules/browser/protocol.rs`,
  `src-tauri/subclave-proxy/src/frame.rs` and
  `src-tauri/src/modules/browser/auth.rs` literal for literal; the browser
  verify script pins the agreement.
- The service worker routes by sender (`senderKind` in `src/background.ts`):
  popup requests only from extension pages, inline requests only from the
  top-frame content script, answered for the sender's own tab URL or, for the
  save prompt, the URL the same tab's submit was stamped with (shown only on a
  page of that site, which Rust decides), at `scope: "host"` and
  `via: "inline"`.
- The inline UI (`src/content/inline.ts`) lives in one closed shadow root
  under `<subclave-inline>`, built with `createElement`/`textContent` (no
  `innerHTML`). Every fill, generate or update pick, and every save-prompt Add
  or Update, passes the six guards in `src/content/guard.ts` first;
  `test/guard.spec.ts` has one attack fixture per guard. `scripts/build.mjs`
  fails a build whose `content.js` is over 30 KB.

### Import and backup (`src-tauri/src/modules/import/`, `src-tauri/src/modules/backup.rs`, `src/modules/backup/`)

- A preview stages the decoded records in `Unlocked.staged`
  (`src-tauri/src/modules/vault/state.rs`), one preview at a time; a new
  preview replaces and wipes the last, and every lock wipes it with the
  payload (`Unlocked::wipe`).
- The webview gets titles, hosts, usernames and counts, never a password.
  The apply takes the preview's `handle`, not the rows themselves.
- An apply lands through `commit` with `mark_dirty` on every changed record
  and emits `subclave:vault-changed` with origin `import`.
- The backup apply merges each record through `merge_into_payload`
  (`src-tauri/src/modules/sync/engine/payload.rs`), the path a sync pull lands
  records through; there is no second merge.
- A backup holds entries and groups only: no `DeviceState`, no tombstones, no
  `lastUsedAt`. Writing one refuses a passphrase under zxcvbn score 3, the
  sync passphrase's rule.
- CSV export uses Bitwarden's columns, so the file re-imports here and into
  Bitwarden.

## Workflow

- CI (`.github/workflows/ci.yml`) runs, frontend job: `pnpm run lint:imports`,
  `pnpm run typecheck:scripts`, `pnpm run format:check`, `pnpm run verify`,
  `pnpm exec tsc --noEmit`, `pnpm run typecheck:extension`, `pnpm build`; rust
  job: `cargo fmt --all -- --check`,
  `cargo check --workspace --all-targets --locked`, a `subclave-proxy` check
  for `x86_64-pc-windows-msvc`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo test --workspace --locked`; extension job: type-check, build both
  targets (`pnpm --filter subclave-extension build --target chrome`, then
  `--target firefox`), `web-ext lint` on the Firefox build, build the
  `subclave-proxy` crate, run the Playwright specs under `extension/test/`,
  and package the zips.
- Docs: `ARCHITECTURE.md` holds the module map and the main flows,
  `KNOWN-LIMITS.md` the accepted limits, `SECURITY.md` the threat model. A
  change that moves a module, a flow, a limit or a trust boundary updates the
  matching file in the same PR.
- `pnpm run verify [substring]` runs `scripts/*-verify.ts`, auto-globbed by
  `scripts/verify-all.mjs` (a new file needs no registration). Modules meant to
  be exercised here stay free of Tauri imports at module scope, or take their
  IO as an injected port, so plain node can load them.
- `pnpm tauri:dev` uses `tauri.dev.conf.json` (`dev.rendy.subclave.dev`),
  isolating stores and logs in a `.dev` data dir.
- Release: feature branch -> PR into `dev` (squash) -> PR `dev` -> `main`
  (merge commit) -> annotated tag `vX.Y.Z` on `main`. The tag (`v*`) triggers
  `.github/workflows/release.yml`, which builds signed updates and a draft
  GitHub Release; notes are generated from `CHANGELOG.md` via
  `scripts/release-notes.mjs`, which reads the heading
  `## [X.Y.Z] - DD-MM-YYYY`. The file holds only the latest release; every
  earlier version lives in GitHub Releases, so a new draft replaces the old
  section instead of stacking on it.

## Gotchas

- `tauri.conf.json` sets `"removeUnusedCommands": true`, so a command with no
  frontend `invoke` call site can be stripped from a release build. Commands
  shipped with no caller are listed in `UNINVOKED`
  (`scripts/command-registry-verify.ts`), which fails if a command loses its
  last caller without an entry, or an entry gains one.
- The Settings window is denylisted from `tauri-plugin-window-state`, and
  `VISIBLE` is stripped from the restored state flags so the main window can
  call `show()` after first paint instead of flashing a transparent shadow.
- `bundle.externalBin` names the `subclave-proxy` sidecar, and `tauri-build`
  checks that `src-tauri/binaries/subclave-proxy-<triple>` exists while the
  build script runs, so a bare `cargo` invocation fails with
  `ResourcePathNotFound` until `pnpm build:sidecar` has staged it (see
  CONTRIBUTING). The sidecar's profile must match the app's: a debug app
  resolves the `.dev` socket name, so `build:sidecar:dev` is what `tauri dev`
  and `tauri build --debug` need.
