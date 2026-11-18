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

Download links stay current in three layers:

1. **At deploy time** the Pages workflow runs `scripts/website-release-links.py`,
   which reads the releases with the workflow token (no visitor quota) and
   rewrites every download link, size and version in the published
   `index.html`. It also writes `releases.json` next to it.
2. **After each release** the `publish` job in `release.yml` starts the Pages
   workflow (`gh workflow run pages.yml`), because a release published with
   `GITHUB_TOKEN` does not trigger other workflows by itself.
3. **In the browser** `main.js` asks the GitHub API for the newest assets, and
   falls back to `releases.json` when the API's 60 requests/hour per network
   are used up.

Assets are matched by file name:

| Button | Asset name pattern |
| --- | --- |
| macOS | `Sanser-macOS-<ver>-arm64.dmg` (and `.zip`) |
| Windows | `Sanser-Windows-<ver>-x64-setup.exe` (and `-portable.exe`) |
| Android | `Sanser-Android-<ver>-arm64[-preview].apk` |
| iOS | `Sanser-iOS-<ver>-device-unsigned.ipa` |

The links in the repository's `index.html` are only a starting point; there is
no need to edit them per release. The "What's new" text still comes from
`docs/releases/` by hand. If a deploy did not happen after a release, run
*Actions → Website (GitHub Pages) → Run workflow*.

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

## Google search

The page has a canonical URL, Open Graph tags, `WebSite`/`SoftwareApplication`
structured data and `sitemap.xml`. A `robots.txt` would have no effect here: Google
only reads it at the host root (`d205-vn.github.io/robots.txt`), not under `/Sanser/`.

To get the site into Google:

1. Open [Google Search Console](https://search.google.com/search-console), add a
   **URL prefix** property for `https://d205-vn.github.io/Sanser/`.
2. Verify with the **HTML tag** method: paste the
   `<meta name="google-site-verification" ...>` tag into `<head>` of
   `index.html`, push, then press *Verify*.
3. *Sitemaps*: submit `sitemap.xml`.
4. *URL inspection*: inspect the home page URL and press *Request indexing*.

Indexing usually takes days to a few weeks. Ranking first for the single word
"Sanser" also depends on other sites using that name; a custom domain and links
to the site (the GitHub repo's *Website* field, release notes) help.
