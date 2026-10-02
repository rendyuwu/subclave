# Autofill release checklist

Run before every release tag. It checks that Subclave fills the login form on
50 popular sites. Filling is the extension's whole job, so nothing here signs
in, and only the last step needs an account.

## How to run

1. **Automated**: `pnpm --filter subclave-extension test:sites`. It builds the
   E2E extension, whose fake app answers every page with the same fake logins,
   then loads each login page in the Sites table three times, fresh each time,
   and fills it once through each path: the inline picker, the popup, and the
   fill command. A path is `ok` when the page's login fields hold the fake
   values: username and password on a one-page form, the username on the first
   page of a two-step login (the password page needs a real account;
   `test/fixtures/two-step.html` covers it). The browser runs headed, because
   bot checks let a visible browser through more often than a headless one; on
   a machine without a display, run it under `xvfb-run -a`. It takes about 15
   minutes, and the run table is written to test-results/sites-run.md under
   `extension/`. Add `--grep "GitHub"` to run one site.
2. **Blocked sites**: a page that shows no login field within 15 seconds (a
   bot check, an error page, or a login page that moved) is `blocked`, and a
   site with no `ok` path and at least one `blocked` one gets `blocked` in its
   Pass cell. Check each of those by hand in Chrome with the app and the
   unpacked Chrome build (the dist/chrome folder that
   `pnpm --filter subclave-extension build --target chrome` writes under
   `extension/`), Settings > Browser > "Chromium browsers" on: add an entry
   with any username and password and the site's login URL, open the login page
   and fill it from the popup. Record `ok` or `miss` in the Popup cell and set
   Pass to `yes` or `no`; a site that blocks a manual visit too is `n/a`.
3. **Real app**: on two sites where you have an account, one with a one-page
   form and one with a two-step login, sign in end to end with the real app and
   the same build, filling through the inline picker. This is the one step that
   goes through `subclave-proxy` and the app's own matching.

- **Pass**: a site passes when at least one path is `ok`.
- **Release gate**: at least 90% of the tested sites pass (45 of 50 when none is
  `n/a`), and both real-app sign-ins work.
- **Swaps**: a site whose login page moved or is gone may be replaced in the
  Sites table before a run; the change is made in the table, not hidden in a
  run.
- **Not in CI**: live pages change and block automated browsers, so
  `test/sites.spec.ts` runs only through `test:sites`
  (`test/sites.config.ts`), never under `pnpm test`.

## Sites

| #   | Site              | Login page                                       |
| --- | ----------------- | ------------------------------------------------ |
| 1   | GitHub            | https://github.com/login                         |
| 2   | GitLab            | https://gitlab.com/users/sign_in                 |
| 3   | Codeberg          | https://codeberg.org/user/login                  |
| 4   | Google            | https://accounts.google.com/                     |
| 5   | Microsoft account | https://login.live.com/                          |
| 6   | Apple Account     | https://account.apple.com/sign-in                |
| 7   | Amazon            | https://www.amazon.com/gp/sign-in.html           |
| 8   | eBay              | https://signin.ebay.com/                         |
| 9   | PayPal            | https://www.paypal.com/signin                    |
| 10  | Facebook          | https://www.facebook.com/login                   |
| 11  | Instagram         | https://www.instagram.com/accounts/login/        |
| 12  | X                 | https://x.com/i/flow/login                       |
| 13  | LinkedIn          | https://www.linkedin.com/login                   |
| 14  | Reddit            | https://www.reddit.com/login/                    |
| 15  | Discord           | https://discord.com/login                        |
| 16  | Twitch            | https://www.twitch.tv/login                      |
| 17  | Netflix           | https://www.netflix.com/login                    |
| 18  | Spotify           | https://accounts.spotify.com/login               |
| 19  | Dropbox           | https://www.dropbox.com/login                    |
| 20  | Box               | https://account.box.com/login                    |
| 21  | Atlassian         | https://id.atlassian.com/login                   |
| 22  | Stack Overflow    | https://stackoverflow.com/users/login            |
| 23  | npm               | https://www.npmjs.com/login                      |
| 24  | PyPI              | https://pypi.org/account/login/                  |
| 25  | Docker Hub        | https://app.docker.com/login                     |
| 26  | DigitalOcean      | https://cloud.digitalocean.com/login             |
| 27  | AWS console       | https://console.aws.amazon.com/                  |
| 28  | Cloudflare        | https://dash.cloudflare.com/login                |
| 29  | Heroku            | https://id.heroku.com/login                      |
| 30  | Hetzner           | https://accounts.hetzner.com/login               |
| 31  | Namecheap         | https://www.namecheap.com/myaccount/login/       |
| 32  | GoDaddy           | https://sso.godaddy.com/                         |
| 33  | Stripe            | https://dashboard.stripe.com/login               |
| 34  | WordPress.com     | https://wordpress.com/log-in                     |
| 35  | Yahoo             | https://login.yahoo.com/                         |
| 36  | Proton            | https://account.proton.me/login                  |
| 37  | Zoom              | https://zoom.us/signin                           |
| 38  | Notion            | https://www.notion.so/login                      |
| 39  | Figma             | https://www.figma.com/login                      |
| 40  | Steam             | https://store.steampowered.com/login/            |
| 41  | Epic Games        | https://www.epicgames.com/id/login               |
| 42  | Adobe             | https://auth.services.adobe.com/en_US/index.html |
| 43  | Pinterest         | https://www.pinterest.com/login/                 |
| 44  | Tumblr            | https://www.tumblr.com/login                     |
| 45  | Wikipedia         | https://en.wikipedia.org/wiki/Special:UserLogin  |
| 46  | Hacker News       | https://news.ycombinator.com/login               |
| 47  | JetBrains Account | https://account.jetbrains.com/login              |
| 48  | OpenAI            | https://auth.openai.com/log-in                   |
| 49  | Booking.com       | https://account.booking.com/sign-in              |
| 50  | Mastodon          | https://mastodon.social/auth/sign_in             |

## Runs

Each run records its date, the build tested (the PR number plus its head SHA,
because the squash merge orphans the branch commit), the browsers used, and the
two real-app sites. The table is the automated run's, with the blocked rows
then filled from the manual check. `Form` is `one page`, `two step` or
`other`; a path cell is `ok`, `miss`, `blocked` or `n/a`; `Pass` is `yes`,
`no`, `blocked` (until the hand check) or `n/a`.

### v0.1.0 (DD-MM-YYYY)

Not run yet.
