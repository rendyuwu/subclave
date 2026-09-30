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
- **Window styling**: macOS gets native traffic lights via an Overlay title
  bar; Linux and Windows are borderless with React `WindowControls`. Windows
  adds `apply_windows_frame_fixes` (main window only, maximize-clamp and
  minimize), `disable_windows_corner_rounding`, and
  `disable_browser_accelerator_keys` so WebView2 does not eat Ctrl+W / Ctrl+R.
- **Docs and prose**: no em-dashes (commas, colons, or parentheses instead).
  No emoji in docs, code, or commits.
- **Comments cite only what a clone can reach**: a file `git ls-files` returns,
  a symbol, an upstream project's public tracker named by project, or a pinned
  dependency's own source (crate, version, symbol, never a line number). Never
  cite this project's own planning docs, issue numbers, section numbers, `/tmp`
  paths, dates, or commit hashes.
  One carve-out: a comment may name a file that is gone when the deletion
  itself is the sentence's subject, in the past tense ("once X was deleted, Y
  became unreachable").
- **Cite a symbol, not a line**: a line number goes stale the moment another
  commit touches that file. `scripts/citation-format-verify.ts` fails on a
  `file:line` inside a comment in `src/`, `src-tauri/src/`, or `scripts/`, and
  on a backticked path that resolves to no file in the tree.

## Area rules

### Modal gate (`src/modules/shortcuts/lib/modalRegistry.ts`)

- No catalogued shortcut fires while a `Dialog`/`AlertDialog` is open. The
  registry is a stack; only the command palette's own chord is exempt, and only
  while the palette is topmost.

## Workflow

- CI (`.github/workflows/ci.yml`) runs, frontend job: `pnpm run lint:imports`,
  `pnpm run typecheck:scripts`, `pnpm run format:check`, `pnpm run verify`,
  `pnpm exec tsc --noEmit`, `pnpm build`; rust job: `cargo fmt --all -- --check`,
  `cargo check --all-targets --locked`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo test --workspace --locked`.
- `pnpm run verify [substring]` runs `scripts/*-verify.ts`, auto-globbed by
  `scripts/verify-all.mjs` (a new file needs no registration). Modules meant to
  be exercised here stay free of Tauri imports at module scope, or take their
  IO as an injected port, so plain node can load them.
- `pnpm tauri:dev` uses `tauri.dev.conf.json` (`dev.rendy.subclave.dev`),
  isolating stores and logs in a `.dev` data dir.
- Release: a tag matching `v*` triggers `.github/workflows/release.yml`, which
  builds signed updates and a draft GitHub Release; notes are generated from
  `CHANGELOG.md` via `scripts/release-notes.mjs`. The file holds only the
  latest release; every earlier version lives in GitHub Releases, so a new
  draft replaces the old section instead of stacking on it.

## Gotchas

- `tauri.conf.json` sets `"removeUnusedCommands": true`, so a command with no
  frontend `invoke` call site can be stripped from a release build. Commands
  shipped with no caller are listed in `UNINVOKED`
  (`scripts/command-registry-verify.ts`), which fails if a command loses its
  last caller without an entry, or an entry gains one.
- The Settings window is denylisted from `tauri-plugin-window-state`, and
  `VISIBLE` is stripped from the restored state flags so the main window can
  call `show()` after first paint instead of flashing a transparent shadow.
