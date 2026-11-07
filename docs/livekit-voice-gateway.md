# Enhanced desktop voice: deployment and acceptance

Enhanced voice uses the pinned LiveKit WebRTC adapter already exercised by
Voice Lab. It is opt-in, not a replacement for every native voice path. It adds
a local Tauri floating view for Mac/Windows. Flutter voice is not implemented by
this increment. No public SFU or user installation was changed during development.

## Admission topology

```text
Desktop voice window / browser lab
  ├─ HTTPS account/group credential request → Sanser API
  ├─ WSS /rtc or /rtc/v1 → Sanser admission gateway → private SFU signaling
  └─ WebRTC media → SFU ICE/UDP or configured TURN
```

The SFU's HTTP/WS port must be bound to `127.0.0.1` and not exposed through a
second proxy, port-forward or public listener. The Sanser API and SFU signaling
run on the same machine/network namespace. Sanser intentionally refuses a
remote/public upstream in enabled gateway configuration. Do not configure a
public direct LiveKit endpoint as the gateway: it would bypass Sanser's live
authorization. Media ports are separate and must remain reachable as required
by the deployment.

Required server settings, using private operator-managed key material:

```dotenv
VOICE_LAB_EXPERIMENTAL_ENABLED=true
VOICE_LAB_LIVEKIT_URL=ws://127.0.0.1:7880
VOICE_LAB_GATEWAY_URL=wss://YOUR_SANSER_API_ORIGIN
VOICE_LAB_LIVEKIT_API_KEY=YOUR_PRIVATE_API_KEY
VOICE_LAB_LIVEKIT_API_SECRET=YOUR_RANDOM_SECRET_AT_LEAST_32_BYTES
```

These are placeholders, not deployable credentials. Keep real values out of the
repository. Publish only the API's `/rtc`, `/rtc/v1`, and corresponding
`/validate` paths over TLS. Never publish `/twirp` or the upstream SFU HTTP port.
Proxy access/error logs must not record query strings on signaling requests:
LiveKit SDKs put their scoped JWT in `access_token`. Sanser traces log the path
only. Use a normal WebSocket proxy timeout rather than an HTTP request-body
timeout for upgraded connections.

WAN audio needs a reachable WebRTC media path. An HTTPS tunnel alone does not
provide UDP media ingress or TURN. Configure and validate the appropriate
ICE/TURN path before enabling this outside LAN. See LiveKit's primary
[deployment](https://docs.livekit.io/transport/self-hosting/deployment/) and
[port/firewall](https://docs.livekit.io/transport/self-hosting/ports-firewall/)
references. The local three-peer test does not validate WAN deployment.

Migrations 0009–0011 provide revision fencing, durable room closure and
login/device-bound admission. Both cached and SFU-refreshed tokens are checked
against current membership/login/presence. The worker retires invalid rounds and
calls `DeleteRoom`; retries survive restart. During SFU/DB failure, termination of
already-connected peers can be delayed. The gateway closes signaling with a
terminal Leave message when authorization cannot be confirmed. Drain pending
closures before changing SFU config/keys. Successfully closed/retired rooms are
retained for 30 days, then deleted in batches of at most 100 with their admission
mappings. Active, pending and leased rows are never pruned by retention.

The [same-host deployment bundle](../deploy/voice/README.md) supplies private
LiveKit signaling, a systemd service and a TLS reverse-proxy example. It has not
provisioned a public server or validated WAN media/TURN connectivity.

## Desktop behavior

- A configured server advertises the authenticated capability; otherwise the
  existing native voice path stays available. Select **Join enhanced voice**.
- Enhanced voice owns one desktop session at a time. Close other host/viewer
  sessions first; replacing an engine while its enhanced call is active is
  blocked until the voice window closes, preventing an ungated audio restart.
- A new native-engine capability is mandatory. Older engines are rejected
  before opening a second voice engine.
- The native worker stops its mic/player, gates system-sound publication and
  acknowledges before the WebRTC window opens. This prevents two capture paths
  and avoids feeding browser voice back into shared system sound.
- The host floating bar combines voice and **Stop sharing**; viewers get voice
  controls only. Native controls yield while enhanced voice owns the call.
- Join is listen-only. Unmute is explicit; Leave/Deafen/reconnect/late callbacks
  cannot restore microphone consent. Closing the window restores shared sound
  and the native sharing bar. Ending the native session closes its voice window.
- A window that has not confirmed a connected voice room within 25 seconds is
  closed by the native monitor on its next two-second check, restoring shared
  sound even if the page never loads or its startup script fails.
- Mic acquisition/publication has a 15-second deadline. Late permission grants
  and late publish completions are stopped/unpublished, including after a retry.
  An ended event from a deliberately stopped old mic cannot mute its replacement.
- Browser device-change events retry playback without opening a mic; blocked
  playback still requires the explicit Play voice audio action. Physical device
  routing and jack changes remain acceptance tests, not presumed successful.
- The grant is handed off once through a window-restricted command, not a URL,
  event broadcast, preference or log. The view cannot invoke account storage,
  UAC or engine commands. Its bundle has only local audio media CSP permissions.

Shared music/system sound pauses during enhanced voice on both platforms. This
is a deliberate fallback, not completed process-specific browser-audio exclusion.
The window uses browser/WebRTC echo cancellation, noise suppression and mono
speech capture; those settings do not prove acoustic quality on a user's device.

## Physical acceptance (pending)

The user currently cannot perform the two-machine trial. Do not mark these done
or publish a release on the strength of synthetic packets alone:

| Case | Required observation |
| --- | --- |
| Mac/Windows, 2 then 3 simultaneous talkers | No crackle, runaway gain, echo or accumulating delay for at least 10 minutes |
| Headphones and speakers | Check each independently; record mic/output type and OS permission state |
| Mac 3.5 mm jack inserted/removed during a call | Playback recovers; no 40-second silence or continuous popping |
| Mute / deafen / close / native session ends | Mic stops; shared sound resumes after closing enhanced voice |
| Floating window and OS microphone prompt | Buttons and permission prompts work without trapping remote keyboard focus |
| Host stops / removes viewer / logs out | Existing call ends and cached/refreshed-token rejoin is denied |
| Direct LAN, WAN and fallback conditions | Measure native media separately from SFU voice; do not call SFU RTT “Wire RTT” |

The local three-peer synthetic Opus sample also ran for 120,045 ms with production
authorization checks and fresh device heartbeats throughout: all three received
two tracks, with 12,170 / 12,128 / 12,127 nonempty RTP packets. The largest observed
userspace packet gaps were 63.487 / 63.333 / 63.312 ms. These are receipt intervals,
not network RTT, decoded sound quality or audible latency. Every track is checked
for a stall longer than three seconds after startup. The short revocation trial
is retained separately, so expiry of a 60-second original grant in the long trial
cannot masquerade as proof of revocation. No real audio device was opened.

Automated evidence is in the dated ignored artifact directory and the
[local SFU test procedure](../apps/voice-lab/testing/README.md). No end-to-end
latency improvement or elimination of audible crackle has been certified yet.
