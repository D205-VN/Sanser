# Desktop updater recovery

The installed Mac application reports 2.3.5 while the public stable release and
signed updater manifest report 2.3.6. A live read of the configured endpoint,
`https://github.com/D205-VN/Sanser/releases/latest/download/updater-latest.json`,
returned HTTP 504 in 12.52 seconds. The GitHub release asset API served the same
public files and their signatures/checksums during release verification.

The Update action remains hidden until Tauri confirms a newer version. It must
not appear merely because a check failed. About gains a separate manual check;
its failure dialog does not claim the installed app is current.

The primary check has an eight-second timeout. On failure, a fixed native
command queries only D205-VN/Sanser's stable GitHub release, then uses the public
asset API to fetch its manifest. Draft/prerelease, duplicate/missing assets,
cross-release installer links and external URLs are rejected. The metadata
response is bounded; requests have timeouts and carry no account token.

The fallback creates Tauri's own Update resource and retains its default semver
comparison, configured public key, signature verification and installer flow.
The selected installer URL is mapped to the same release's public asset API,
also avoiding the failing browser download endpoint. No unsigned install or
custom installer execution path is introduced.

Failed checks retry after 30 seconds with backoff capped at five minutes.
Returning to the app can check again after five minutes; focus bursts, overlapping
requests and unmounted components cannot keep checking. Periodic checks and
online recovery remain supported. Installation/restart remain blocked during
an active remote session.

This is a source patch following public 2.3.6. It does not modify the installed
2.3.5 app or replace existing public 2.3.6 assets. An older application whose
only endpoint is failing needs one manual installation of a build containing
the fallback before it can use the new recovery path.
