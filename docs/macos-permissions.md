# Mac hosting permission and signing

Normal Windows-to-Mac control and multi-viewer sharing both use the Mac native
host and the Screen Recording permission of `com.sanser.desktop`.

## Confirmed stale-grant failure

On 2026-10-14, the installed 2.2.2 bundle passed `codesign --verify --deep
--strict`, but macOS TCC reported:

```text
Failed to match existing code requirement for subject com.sanser.desktop
and service kTCCServiceScreenCapture
```

The stored requirement and running app had different `cdhash` values. The
permission switch still appeared enabled. TCC correctly attributed the native
host request to the parent Sanser app; this was not a missing Windows permission
or a separate multi-viewer transport failure.

An ad-hoc signature uses a build-specific identity. Replacing that build can
invalidate an existing grant. ScreenCaptureKit startup fixes cannot repair this
stored macOS authorization record.

## Recover the installed app

1. Quit Sanser on the host Mac.
2. In System Settings → Privacy & Security → Screen & System Audio Recording,
   remove the old Sanser entry. If necessary, reset only its recording grant:
   `tccutil reset ScreenCapture com.sanser.desktop`.
3. Use **+** to add `/Applications/Sanser.app` and allow recording.
4. Fully quit and reopen Sanser, then reconnect. Remote keyboard/mouse control
   additionally needs Accessibility; view-only sharing does not.

Resetting permission never grants access. The user must approve the current app
in macOS. Do not reset all applications' permissions or edit the TCC database.

## Stable signing

For local development, keep using the same **Apple Development** certificate and
bundle ID when building. Set `APPLE_SIGNING_IDENTITY` to the identity listed by
`security find-identity -v -p codesigning`. A locally development-signed build is
not a notarized public release. Reinstalling an ad-hoc GitHub artifact changes
the identity again and may require a fresh grant.

For public releases, configure these GitHub Actions repository secrets:

- `APPLE_CERTIFICATE`: base64 of the Developer ID Application `.p12` certificate
  and private key, exported by its owner.
- `APPLE_CERTIFICATE_PASSWORD`: its export password.
- `APPLE_SIGNING_IDENTITY`: the full `Developer ID Application: …` identity.

For notarization, also configure `APPLE_ID`, `APPLE_PASSWORD` (an app-specific
password), and `APPLE_TEAM_ID`. The release workflow rejects partial signing or
notarization configuration. If no Apple signing secrets exist, it explicitly
warns and produces the existing ad-hoc build. Merely adding the workflow support
does not make existing or subsequent releases Developer ID signed.

Keep certificate/private-key material in the Keychain or GitHub Secrets, never
in the repository or logs. Updater signing is separate from Apple code signing;
the updater signature alone does not provide a stable macOS permission identity.

References: [Apple code-signing requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements),
[Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/), and
[Apple screen-recording settings](https://support.apple.com/guide/mac-help/control-access-screen-system-audio-recording-mchld6aa7d23/mac).
