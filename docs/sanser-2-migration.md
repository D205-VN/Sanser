# Sanser 1.x to 2.0 migration

Migration is transactional and never deletes the old data directory before verification.

1. Detect the legacy data directory without starting legacy runtime code.
2. Create a timestamped backup.
3. Import safe preferences, server URL, display/audio choices, and a stable device ID only when its format validates.
4. Map Tailscale network settings to `Auto`; map its quality profile to `Balanced` or `Internet`.
5. Do not import legacy access tokens, TURN credentials, database credentials, or cryptographic session material.
6. Write migration version `2` and commit the SQLite transaction.
7. Preserve the backup and report individual skipped fields without logging their values.

## Legacy feature parity inventory

The previous application included account auth, device presence, manual approval, WebRTC screen/audio, native Windows capture/encode, macOS VideoToolbox/Metal render, UDP/TCP media, PCM audio, keyboard/mouse/gamepad, feedback/NACK, diagnostics, and LAN discovery. Sanser 2 keeps these behaviors behind new typed boundaries; it does not reuse the Electron/Node runtime architecture.

Legacy source remains only while a milestone still needs behavioral reference. It is excluded from Sanser 2 packaging and is removed once replacement tests pass.
