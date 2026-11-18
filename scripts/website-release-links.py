#!/usr/bin/env python3
"""Bake the newest GitHub release download links into the built website.

Usage: website-release-links.py <site_dir>

Reads the repository's releases from the GitHub API (GITHUB_TOKEN and
GITHUB_REPOSITORY from the environment), writes <site_dir>/releases.json for
main.js, and rewrites the fallback links/versions in <site_dir>/index.html so
visitors get current links even without JavaScript or GitHub API quota.
"""
import json
import os
import re
import sys
import urllib.request

ASSETS = [
    # (element id, asset name pattern, meta element id)
    ("dl-mac", r"^Sanser-macOS-[\d.]+-arm64\.dmg$", "dl-mac-meta"),
    ("dl-mac-zip", r"^Sanser-macOS-[\d.]+-arm64\.zip$", None),
    ("dl-win", r"^Sanser-Windows-[\d.]+-x64-setup\.exe$", "dl-win-meta"),
    ("dl-win-portable", r"^Sanser-Windows-[\d.]+-x64-portable\.exe$", None),
    ("dl-android", r"^Sanser-Android-[\d.]+-arm64(-preview)?\.apk$", "dl-android-meta"),
    ("dl-ios", r"^Sanser-iOS-[\d.]+-device-unsigned\.ipa$", "dl-ios-meta"),
]


def fetch_releases(repo, token):
    req = urllib.request.Request(
        f"https://api.github.com/repos/{repo}/releases?per_page=30",
        headers={"Accept": "application/vnd.github+json", "Authorization": f"Bearer {token}"},
    )
    with urllib.request.urlopen(req, timeout=30) as res:
        data = json.load(res)
    return [
        {
            "tag_name": r["tag_name"],
            "draft": r["draft"],
            "prerelease": r["prerelease"],
            "assets": [
                {"name": a["name"], "size": a["size"], "browser_download_url": a["browser_download_url"]}
                for a in r.get("assets", [])
            ],
        }
        for r in data
        if not r["draft"] and not r["prerelease"]
    ]


def newest(releases, pattern):
    rx = re.compile(pattern, re.I)
    for r in releases:
        for a in r["assets"]:
            if rx.match(a["name"]):
                return r, a
    return None, None


def main():
    site = sys.argv[1]
    releases = fetch_releases(os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_TOKEN"])
    with open(os.path.join(site, "releases.json"), "w") as f:
        json.dump(releases, f, separators=(",", ":"))

    path = os.path.join(site, "index.html")
    html = open(path, encoding="utf-8").read()
    desktop_version = None
    for el_id, pattern, meta_id in ASSETS:
        rel, asset = newest(releases, pattern)
        if not asset:
            continue
        version = rel["tag_name"].lstrip("v")
        html = re.sub(rf'(id="{el_id}" href=")[^"]*', rf'\g<1>{asset["browser_download_url"]}', html)
        if meta_id:
            meta = f'v{version} · {asset["size"] / 1048576:.1f} MB'
            html = re.sub(rf'(id="{meta_id}">)[^<]*', rf"\g<1>{meta}", html)
        if el_id == "dl-mac":
            desktop_version = version
            sums = next((a for a in rel["assets"] if a["name"] == "SHA256SUMS.txt"), None)
            if sums:
                html = re.sub(r'(id="dl-sums" href=")[^"]*', rf'\g<1>{sums["browser_download_url"]}', html)
    if desktop_version:
        html = re.sub(r'(class="js-desktop-version">)[^<]*', rf"\g<1>{desktop_version}", html)
        html = re.sub(r'("softwareVersion": ")[^"]*', rf"\g<1>{desktop_version}", html)
    open(path, "w", encoding="utf-8").write(html)
    print(f"desktop {desktop_version}; releases.json has {len(releases)} releases")


if __name__ == "__main__":
    main()
