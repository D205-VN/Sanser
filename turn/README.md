# Sanser TURN Server

Use this on a machine with a reachable public IPv4 address when Sanser needs to connect across different networks.

## Option A: Run directly on macOS

Install coturn:

```bash
brew install coturn
```

Copy the example:

```bash
cp turnserver.conf.example turnserver.conf
```

Edit `turnserver.conf`:

```text
external-ip=YOUR_PUBLIC_IP
realm=YOUR_PUBLIC_IP_OR_DOMAIN
user=your_username:your_password
```

Start TURN:

```bash
/opt/homebrew/opt/coturn/bin/turnserver -c turnserver.conf -o --pidfile /tmp/sanser-turn.pid --log-file /tmp/sanser-turn.log
```

Stop TURN:

```bash
kill "$(cat /tmp/sanser-turn.pid)"
```

## Option B: Run with Docker

Docker is useful on a VPS, but it is not required on macOS.

On Ubuntu, install Docker:

```bash
sudo apt update
sudo apt install -y docker.io docker-compose-plugin
sudo systemctl enable --now docker
```

Then configure `turnserver.conf` as above and start:

```bash
docker compose up -d
docker logs -f sanser-coturn
```

## Required ports

Open or forward these ports to the TURN machine:

```text
3478 TCP
3478 UDP
49152-65535 UDP
```

If you run TURN on a home Mac, configure port forwarding on the router to the Mac LAN IP.

## Configure Sanser

On every Sanser app server that clients use, set:

```env
STUN_URLS=stun:stun.l.google.com:19302
TURN_URLS=turn:YOUR_PUBLIC_IP_OR_DOMAIN:3478
TURN_USERNAME=your_username
TURN_CREDENTIAL=your_password
```

Restart Sanser on both machines after changing `.env`.

## Notes

If the VPS has a domain and TLS certificate, you can later add `turns:YOUR_DOMAIN:5349`, but plain TURN on `3478` is enough to prove cross-network connectivity first.
