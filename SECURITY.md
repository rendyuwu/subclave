# Security

## Reporting

File a private report through
**[GitHub Security Advisories](https://github.com/rendyuwu/subclave/security/advisories/new)**.
Do not open a public issue. Include what it lets an attacker do, steps to
reproduce, and version, OS, arch. Fixed reports are credited in the release
notes unless you ask otherwise.

Until `1.0.0`, only the latest minor gets security fixes.

## Scope

- In: the Rust backend (`src-tauri/`, including the `subclave-proxy` sidecar), the frontend, the browser extension (`extension/`), the update feed and release signatures.
- Out: bugs in upstream dependencies (Tauri, browsers), report those upstream. The out-of-scope attackers listed under Threat model.

## Threat model

### In scope

- **Web pages.** A malicious or compromised site must not trigger a fill,
  get a credential for another site, or read the vault.
  - The content script runs in the top frame only. It never reads extension
    storage. It holds a credential only after a guarded pick, and reads the
    login the user typed only on a submit that is trusted and follows a user
    gesture.
  - The service worker keeps that typed pair in `chrome.storage.session`,
    for its tab only, until the save prompt is answered, the tab closes, the
    tab's third page load passes, or the first page load after 5 minutes.
    The save URL is the browser-stamped `MessageSender.url` of the submit.
    The prompt shows only on a page of the submit's site: Rust compares the
    asking page's `MessageSender.url` with the submit's URL by the Domain rule
    (`check_login`). `check-login` sees the submitted host's exact-host
    entries only and never returns a password. Add and Update pass the six
    guards and save only the pair the prompt shows: the pair's id goes back
    with the click, and a new submit closes a prompt that is showing.
    Residual risk:
    - After a user gesture, a page can plant values in its own form and
      submit them. It can then learn whether a planted pair matches an entry
      for its own exact host (whether the prompt shows), and offer the user an
      Update the user must still click; the overwritten password stays in the
      entry's history.
    - When the tab moves to another host of the same site (`login.example.com`
      to `www.example.com`), that host's page shows the prompt with the
      submitted host's entry titles and usernames, inside the closed shadow
      root. Its Add and Update can only save the pair the user typed to the
      submitted host.
  - The inline picker offers only exact-host matches. The scheme and port
    rules still apply.
    Wider matches (subdomains, parent domain) are reachable only from
    browser-owned UI: the popup and the keyboard command. A script on
    `cdn.example.com` therefore cannot clickjack its way to the
    `example.com` login, which was the main vector in Marek Tóth's DEF CON 33
    research
    ([DOM-based Extension Clickjacking](https://marektoth.com/blog/dom-based-extension-clickjacking)).
  - Inline fill needs a trusted user action and passes the six guards in
    `extension/src/content/guard.ts`. If any guard fails, nothing is filled.
  - The page never supplies the URL. It comes from the browser:
    `MessageSender.url` for inline, the active tab for the popup and the
    command.
  - Rust re-checks the match before it releases a password, and enforces the
    exact-host rule for inline requests itself.
- **Other OS users and the network.**
  - No TCP listener.
  - The socket lives in a 0700 per-user directory, and the app checks the
    peer's uid.
  - The Windows pipe has a current-user DACL, is created as the first
    instance, and rejects remote clients.
- **Whoever holds the sync storage.**
  - They see ciphertext and opaque names only.
  - They cannot read or forge records: AES-256-GCM, HMAC object names, and
    `kind`/`id` checked on open (`MergeError::IdentityMismatch` in
    `src-tauri/src/modules/sync/model.rs`).
  - They can delete objects, withhold updates, or serve an older copy to a
    device that never saw the newer one. Devices keep their local copy, and
    backups exist for this case.
- **Anyone who gets the vault file, its `.bak`, an OS backup, or a
  `.subclave-backup`.** They can only brute-force it, and Argon2id slows that
  down.
  - A `.subclave-backup` is sealed with Argon2id and AES-256-GCM, its header
    is the associated data, and a weak passphrase is refused when it is
    written.
- **A person at an unlocked, unattended machine.** Auto-lock and clipboard
  clearing limit how long the vault is exposed.

### Out of scope

- **Malware running as the user.** It can:
  - log the master password;
  - read browser and app memory;
  - read the extension's pairing secret from the browser profile;
  - replace binaries.

  No desktop password manager defends against this, and Tervia puts it out of
  scope too. The pairing secret still stops a generic process that only
  speaks the protocol from pulling logins.

- **A compromised OS, browser, or extension store.**
- **Hibernation.** A machine hibernated while the vault is unlocked writes the
  key to disk. Full-disk encryption covers this.
- **Old vault copies.** Backups, and `.bak` files from before a master
  password change, still open with the password that was current when they
  were written.
- **Script injected into the webview.** The rule that the webview gets secrets
  only on reveal is enforced by UI code and the CSP, not by Rust. A script
  running in the webview could call `vault_entry_reveal`, and could call
  `export_csv` or `backup_export` to write the vault to any path.
- **Page script on the exact host where a credential is stored.** An XSS on
  `github.com` itself can try to defeat the inline guards. They are
  JavaScript against JavaScript, and Tóth's research states there is no
  simple complete protection. Such a script can already read any fill made on
  that page, popup fills included. The residual risk is an unintended fill on
  a host the attacker already controls. Anyone who wants to rule that out can
  switch off "Show in login fields" and use the popup.
- **Plaintext exports.** A CSV export holds every password in plain text;
  anyone who gets the file reads them. Deleting it from the import dialog is a
  normal file delete, not a secure erase.
