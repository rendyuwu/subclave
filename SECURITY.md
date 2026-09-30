# Security

## Reporting

File a private report through
**[GitHub Security Advisories](https://github.com/rendyuwu/subclave/security/advisories/new)**.
Do not open a public issue. Include what it lets an attacker do, steps to
reproduce, and version, OS, arch. Fixed reports are credited in the release
notes unless you ask otherwise.

Until `1.0.0`, only the latest minor gets security fixes.

## Scope

- In: the Rust backend (`src-tauri/`), the frontend, the update feed and release signatures.
- Out: bugs in upstream dependencies (Tauri), report those upstream. Attacks that need an already-compromised machine or local shell access.
