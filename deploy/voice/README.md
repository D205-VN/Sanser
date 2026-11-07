# Same-host voice deployment bundle (operator configuration required)

These files prepare a Linux public-server deployment; they do not provision a
server, domain, certificate, key, firewall or TURN and have not been deployed to
the Internet. The user currently has no public UDP server. Never treat copying
these files as completed WAN validation. Desktop physical acceptance remains in
[the main checklist](../../docs/livekit-voice-gateway.md).

Use the same machine/network namespace for Sanser API and LiveKit 1.13.11.
The existing account database stays configured through the API's DATABASE_URL;
this bundle does not create, migrate or replace a production database by itself.
Build Sanser API with its normal migrations and install the pinned official
LiveKit binary at /usr/local/bin/livekit-server. Create a dedicated unprivileged
sanser-voice user/group, then place livekit.yaml and a private keys.yaml under
/etc/sanser-voice. The latter maps the private API key to a random secret of at
least 32 bytes; own the directory/files for that service and use mode 0700/0600.
Keep keys outside this repository. Do not put secrets in ExecStart arguments.

Configure the API privately on 127.0.0.1:5174, with its existing DATABASE_URL,
SESSION_CREDENTIAL_KEY and ALLOWED_ORIGINS. Add:

```dotenv
VOICE_LAB_EXPERIMENTAL_ENABLED=true
VOICE_LAB_LIVEKIT_URL=ws://127.0.0.1:7880
VOICE_LAB_GATEWAY_URL=wss://YOUR_API_DOMAIN
VOICE_LAB_LIVEKIT_API_KEY=YOUR_PRIVATE_KEY_NAME
VOICE_LAB_LIVEKIT_API_SECRET=THE_SAME_PRIVATE_SECRET_AS_KEYS_YAML
```

Use a trusted TLS certificate and adapt nginx.conf.example to the API domain.
Validate with nginx -t on that Linux host before reload. The proxy forwards to
Sanser admission, never directly to LiveKit. The example disables request logs
for this vhost because signaling queries contain tokens. CDN/WAF/proxy logs
upstream of nginx need the same treatment. Numeric Sanser worker logs remain.

Install the reviewed systemd unit; run daemon-reload and enable/start it only
after private keys and firewall rules are correct. The SFU's HTTP 7880 must
listen on 127.0.0.1 only. API HTTP 5174 is also private. With this baseline,
public TCP 443 serves HTTPS/WSS, TCP 7881 serves ICE fallback, and UDP 7882 serves
ICE mux. Cloud security groups and the host firewall both need the media ports.
The SFU's HTTP bind setting is distinct from RTC media sockets in the pinned
server; do not expose HTTP to try to repair media connectivity.

Restricted networks may require additional TURN/TLS on TCP 443 on a separate
public IP or a properly configured layer-4 endpoint. An HTTPS reverse proxy
cannot proxy TURN or turn a Render HTTP service into UDP ingress. Use LiveKit's
official [deployment](https://docs.livekit.io/transport/self-hosting/deployment/)
and [port reference](https://docs.livekit.io/transport/self-hosting/ports-firewall/)
for TURN certificates, advertised IPs and firewall setup. This baseline deliberately
does not claim restricted Wi-Fi support without a configured/tested TURN path.

Before enabling the desktop capability publicly:

1. From another network, confirm /api/v2/health is healthy and /rtc/validate
   rejects a request without a grant. /twirp/, /agent and /debug/ are not served.
2. Confirm there is no public listener, alternate proxy or port-forward to
   LiveKit HTTP 7880 or API HTTP 5174. Check both IPv4 and IPv6.
3. Run the scoped three-peer trial over WSS/WebRTC on the deployed host, then
   stop/logout and verify both cached and SFU-refreshed rejoin denial.
4. Run 2–3-person physical speech, output/jack changes and long-session tests.
5. Record Direct/Relay video route separately from SFU voice media routing.

This bundle's port/bind settings were checked with the pinned server's config/
ports parser locally, disabling external-IP discovery and using loopback for
that offline dry-run. The production external-IP/STUN discovery was not verified.
Linux systemd execution, nginx syntax with real certificates, public ICE/TURN
reachability and acoustic quality still require deployment/device validation.
