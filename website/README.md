# Sanser website

Static marketing and download site for Sanser. Plain HTML/CSS/JS with no build step.
Vietnamese is the default language; the **EN** button (or `?lang=en`) switches to English.
Light/dark follows the system, and the toggle button saves the choice in the browser.

```text
website/
  index.html        page content (Vietnamese text lives here)
  styles.css        design tokens (light + dark) and layout
  main.js           English strings, theme/lang toggles, OS detection, live release links
  assets/           app icon (from apps/desktop/src-tauri/icons) and mobile UI mockups
                    (from docs/design/mobile-v1/screens, illustrative data)
```

## Download links

Each button has a working fallback `href` pinned to a real GitHub release asset
(desktop `v2.3.15`, mobile `v2.3.7`, the latest release that has APK/IPA files).
On load, `main.js` reads `https://api.github.com/repos/D205-VN/Sanser/releases`
and swaps in the newest matching asset by file name:

| Button | Asset name pattern |
| --- | --- |
| macOS | `Sanser-macOS-<ver>-arm64.dmg` (and `.zip`) |
| Windows | `Sanser-Windows-<ver>-x64-setup.exe` (and `-portable.exe`) |
| Android | `Sanser-Android-<ver>-arm64[-preview].apk` |
| iOS | `Sanser-iOS-<ver>-device-unsigned.ipa` |

So a new release appears on the site without editing it. If the API is
unreachable or rate-limited (60 requests/hour per visitor IP), the pinned links
are used. After a release, update the fallback `href`s and the `v2.3.15`
text in `index.html` so no-JS visitors also get the new version, and refresh the
"What's new" section from `docs/releases/`.

## Preview locally

From the repository root:

```bash
npx --yes serve website
```

or `python3 -m http.server 4173 --directory website`, then open the printed URL.

## Deploy

The site is deployed to **GitHub Pages** by
[`.github/workflows/pages.yml`](../.github/workflows/pages.yml). It runs on a push
to `main` or `Version-2.0.0` that changes `website/**` (or the workflow itself),
and can be started by hand from *Actions → Website (GitHub Pages) → Run workflow*.
It publishes `website/` without this README.

One-time setup in the repository on GitHub:

1. *Settings → Pages → Build and deployment → Source*: choose **GitHub Actions**.
2. *Settings → Environments → github-pages → Deployment branches*: GitHub
   usually limits this to the default branch (`main`). Add `Version-2.0.0` if you
   deploy from that branch; otherwise the deploy job is rejected.
3. Push the `website/` folder and the workflow (or run it by hand).

The site is then served at `https://d205-vn.github.io/Sanser/`. All paths are
relative, so the `/Sanser/` sub-path works. For a custom domain, set it under
*Settings → Pages → Custom domain* and add the DNS record GitHub shows.

Other static hosts also work if you ever move; upload `website/` as the site root:

- **Cloudflare Pages / Netlify / Vercel:** connect the repository, set the
  output directory to `website`, no build command.
- **Render** (already used for the server, see `render.yaml`): add a *Static
  Site* service with publish directory `website` and no build command.

A custom domain needs a DNS record at your registrar plus the host's custom
domain setting.

## Editing rules

- Don't add download links, prices, user counts, testimonials or reviews that
  don't exist. Every feature claim comes from `README.md` and `docs/releases/`.
- Vietnamese text: edit `index.html`. English text: edit the `EN` table in
  `main.js` using the same `data-i18n` key.
