# Autofill release checklist

Run before every release tag, on Google Chrome stable, with the release
candidate: the app plus the unpacked Chrome build of the extension (the
dist/chrome folder that `pnpm --filter subclave-extension build --target chrome`
writes under `extension/`; the `key` in `extension/manifest.chrome.json` gives
it the store ID), with Settings > Browser > "Chromium browsers" on.

- **Entries**: one entry per site with a real account. Its URL is the login
  page's origin from the Sites table, and its match mode is left unchanged.
- **Paths**: per site, from a fresh page load each time:
  - Inline: click the Subclave icon in the username field and pick the entry.
    On a two-step login, repeat on step two.
  - Popup: click the toolbar button and pick the entry.
  - Command: press `Ctrl+Shift+L` (`Cmd+Shift+L` on macOS).
- **Pass**: a path is `ok` when the username and the password both fill (on a
  two-step login, each on its step). A site passes when at least one path is
  `ok`.
- **Release gate**: at least 45 of the 50 sites pass.
- **Swaps**: a site where no account exists may be swapped for another popular
  login page before a run. The swap is made in the Sites table, not hidden in
  a run.

## Sites

| #   | Site                   | Login page                                         |
| --- | ---------------------- | -------------------------------------------------- |
| 1   | GitHub                 | https://github.com/login                           |
| 2   | GitLab                 | https://gitlab.com/users/sign_in                   |
| 3   | Codeberg               | https://codeberg.org/user/login                    |
| 4   | Google                 | https://accounts.google.com/                       |
| 5   | Microsoft account      | https://login.live.com/                            |
| 6   | Apple Account          | https://account.apple.com/sign-in                  |
| 7   | Amazon                 | https://www.amazon.com/ (Account & Lists, Sign in) |
| 8   | eBay                   | https://signin.ebay.com/                           |
| 9   | PayPal                 | https://www.paypal.com/signin                      |
| 10  | Facebook               | https://www.facebook.com/login                     |
| 11  | Instagram              | https://www.instagram.com/accounts/login/          |
| 12  | X                      | https://x.com/i/flow/login                         |
| 13  | LinkedIn               | https://www.linkedin.com/login                     |
| 14  | Reddit                 | https://www.reddit.com/login/                      |
| 15  | Discord                | https://discord.com/login                          |
| 16  | Twitch                 | https://www.twitch.tv/login                        |
| 17  | Netflix                | https://www.netflix.com/login                      |
| 18  | Spotify                | https://accounts.spotify.com/login                 |
| 19  | Dropbox                | https://www.dropbox.com/login                      |
| 20  | Box                    | https://account.box.com/login                      |
| 21  | Atlassian              | https://id.atlassian.com/login                     |
| 22  | Stack Overflow         | https://stackoverflow.com/users/login              |
| 23  | npm                    | https://www.npmjs.com/login                        |
| 24  | PyPI                   | https://pypi.org/account/login/                    |
| 25  | Docker Hub             | https://login.docker.com/                          |
| 26  | DigitalOcean           | https://cloud.digitalocean.com/login               |
| 27  | AWS console            | https://signin.aws.amazon.com/                     |
| 28  | Cloudflare             | https://dash.cloudflare.com/login                  |
| 29  | Heroku                 | https://id.heroku.com/login                        |
| 30  | Hetzner                | https://accounts.hetzner.com/login                 |
| 31  | Namecheap              | https://www.namecheap.com/myaccount/login/         |
| 32  | GoDaddy                | https://sso.godaddy.com/                           |
| 33  | Stripe                 | https://dashboard.stripe.com/login                 |
| 34  | WordPress.com          | https://wordpress.com/log-in                       |
| 35  | Yahoo                  | https://login.yahoo.com/                           |
| 36  | Proton                 | https://account.proton.me/login                    |
| 37  | Zoom                   | https://zoom.us/signin                             |
| 38  | Notion                 | https://www.notion.so/login                        |
| 39  | Figma                  | https://www.figma.com/login                        |
| 40  | Steam                  | https://store.steampowered.com/login/              |
| 41  | Epic Games             | https://www.epicgames.com/id/login                 |
| 42  | Adobe                  | https://account.adobe.com/                         |
| 43  | Pinterest              | https://www.pinterest.com/login/                   |
| 44  | Tumblr                 | https://www.tumblr.com/login                       |
| 45  | Wikipedia              | https://en.wikipedia.org/wiki/Special:UserLogin    |
| 46  | Hacker News            | https://news.ycombinator.com/login                 |
| 47  | JetBrains Account      | https://account.jetbrains.com/login                |
| 48  | OpenAI                 | https://auth.openai.com/log-in                     |
| 49  | Booking.com            | https://account.booking.com/sign-in                |
| 50  | Nextcloud (own server) | `https://<your server>/login`                      |

## Runs

Each run records the browser and its version, the OS, and the build tested:
the PR number plus its head SHA, because the squash merge orphans the branch
commit. `Form` is `one page`, `two step` or `other`; a path cell is `ok`,
`miss` or `n/a`; `Pass` is `yes` or `no`. Misses go in Notes.

### v0.1.0 (DD-MM-YYYY)

- Browser and version:
- OS:
- Build tested (PR and head SHA):

| #   | Form | Inline | Popup | Command | Pass | Notes |
| --- | ---- | ------ | ----- | ------- | ---- | ----- |
| 1   |      |        |       |         |      |       |
| 2   |      |        |       |         |      |       |
| 3   |      |        |       |         |      |       |
| 4   |      |        |       |         |      |       |
| 5   |      |        |       |         |      |       |
| 6   |      |        |       |         |      |       |
| 7   |      |        |       |         |      |       |
| 8   |      |        |       |         |      |       |
| 9   |      |        |       |         |      |       |
| 10  |      |        |       |         |      |       |
| 11  |      |        |       |         |      |       |
| 12  |      |        |       |         |      |       |
| 13  |      |        |       |         |      |       |
| 14  |      |        |       |         |      |       |
| 15  |      |        |       |         |      |       |
| 16  |      |        |       |         |      |       |
| 17  |      |        |       |         |      |       |
| 18  |      |        |       |         |      |       |
| 19  |      |        |       |         |      |       |
| 20  |      |        |       |         |      |       |
| 21  |      |        |       |         |      |       |
| 22  |      |        |       |         |      |       |
| 23  |      |        |       |         |      |       |
| 24  |      |        |       |         |      |       |
| 25  |      |        |       |         |      |       |
| 26  |      |        |       |         |      |       |
| 27  |      |        |       |         |      |       |
| 28  |      |        |       |         |      |       |
| 29  |      |        |       |         |      |       |
| 30  |      |        |       |         |      |       |
| 31  |      |        |       |         |      |       |
| 32  |      |        |       |         |      |       |
| 33  |      |        |       |         |      |       |
| 34  |      |        |       |         |      |       |
| 35  |      |        |       |         |      |       |
| 36  |      |        |       |         |      |       |
| 37  |      |        |       |         |      |       |
| 38  |      |        |       |         |      |       |
| 39  |      |        |       |         |      |       |
| 40  |      |        |       |         |      |       |
| 41  |      |        |       |         |      |       |
| 42  |      |        |       |         |      |       |
| 43  |      |        |       |         |      |       |
| 44  |      |        |       |         |      |       |
| 45  |      |        |       |         |      |       |
| 46  |      |        |       |         |      |       |
| 47  |      |        |       |         |      |       |
| 48  |      |        |       |         |      |       |
| 49  |      |        |       |         |      |       |
| 50  |      |        |       |         |      |       |

Result: N/50 (gate: 45).
