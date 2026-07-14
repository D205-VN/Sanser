# Performance

See [Sanser 2 performance contract](sanser-2-performance.md).
## Relay performance

Auto mode prefers direct UDP for the lowest latency and server bandwidth. When
that route fails, the WSS relay keeps the session reachable through CGNAT at the
cost of an additional network hop and TCP head-of-line blocking. Use the
Competitive or Balanced quality profile on relay routes and place the relay
close to both endpoints. The server processes both ingress and egress traffic,
while packet encryption/decryption remains on the desktops.
