# Wake-on-LAN and connection history

## Wake-on-LAN

Sanser can wake a sleeping or powered-off computer of your account by sending
the standard Wake-on-LAN *magic packet* (6 bytes of `0xFF` followed by the
target adapter's MAC address repeated 16 times) as a UDP broadcast on ports
**9** and **7**.

### How it works

1. **Opt in on the computer you want to wake.** Settings → Host → *Allow
   Wake-on-LAN* (off by default). While Host is online, Sanser reports that
   computer's wired and Wi-Fi adapters to your account: MAC address, private
   IPv4 address, prefix length and the subnet's broadcast address. VPN,
   virtual and loopback adapters are skipped. Turning the option off removes
   the saved details (immediately when Host is online, otherwise the next time
   it comes online).
2. **Wake it from another computer on the same LAN.** On *Computers*, an
   offline computer with saved adapters shows **Wake** instead of *Connect*.
   The desktop app checks that one of its own adapters is on the same IPv4
   subnet, then broadcasts the packet from that adapter to the subnet
   broadcast address and to `255.255.255.255`, on ports 9 and 7.
3. The woken computer comes back in the list once Sanser starts and Host goes
   online (enable *Bring host online automatically* and start Sanser at login).

**Same LAN only.** Routers do not forward broadcasts, so the requesting
computer must be on the same local network (same subnet) as the sleeping one.
From another network the app does not send anything and says so.

### Prerequisites on the computer to be woken

Wake-on-LAN is a hardware/firmware feature; Sanser can only send the packet.

- **BIOS/UEFI:** enable *Wake on LAN*, *Power On By PCI-E/PCI*, or *Resume by
  LAN* (names vary). Disable *ErP/EuP Ready* / *Deep Sleep* modes, which cut
  power to the network adapter. Wake from full shutdown (S5) is supported by
  many desktop boards but not all laptops.
- **Wired Ethernet is the most reliable.** Most Wi-Fi adapters cannot wake
  from shutdown or hibernation; some support wake from sleep only.
- **Windows:** Device Manager → Network adapter → Properties →
  - *Power Management*: tick *Allow this device to wake the computer* and
    *Only allow a magic packet to wake the computer*.
  - *Advanced*: set *Wake on Magic Packet* = Enabled (and *Shutdown Wake-On-Lan*
    if present).
  - Turn off *Fast startup* (Control Panel → Power Options → *Choose what the
    power buttons do*) if waking from shutdown does not work; fast startup
    hibernates instead of shutting down.
- **macOS:** System Settings → Energy (or Battery → Options) → *Wake for network
  access*. Macs wake from sleep, not from shutdown. Apple silicon laptops wake
  only while on power.
- **Network:** the computer must stay connected to the same switch/access point
  while asleep. Some managed switches, mesh Wi-Fi systems or guest networks
  block broadcast traffic; the computers must not be on an isolated guest VLAN.
- **Firewall:** no inbound rule is needed on the sleeping computer (the network
  adapter handles the packet before the OS). The sending computer only needs to
  be able to send outbound UDP broadcasts.

### Privacy

MAC and LAN addresses are only stored when you opt in, are visible only to your
own account, are never written to the audit log, and are removed when you turn
the option off or remove the computer.

### Why there is no relay through another Sanser computer

Waking from outside the LAN would need an online Sanser computer on that LAN to
broadcast the packet on request. Sanser hosts today keep no persistent
server-push channel (signaling sockets exist only per session, and hosts poll
for connection requests), so a relay would need a new always-on channel or
another polling loop on every host. That does not fit the existing signaling
cleanly, so it is intentionally not implemented. Workaround: connect to any
online computer on that LAN with Sanser and press **Wake** from there.

## Connection history

*History* in the sidebar lists the last 90 days of connections in your account:
who connected (requester computer), to which computer (host), when, for how
long, view-only or control, whether media went direct or via the Sanser relay,
and how it ended. Filters narrow it to this computer, connections to this
computer, or connections from this computer.

- **Storage:** history is read from the existing `connection_sessions` table.
  The server marks `relay_used_at` when a session opens the relay; an accepted
  session that never used the relay is shown as *Direct*.
- **Retention:** ended sessions are deleted 90 days after they end, and each
  account keeps at most 1,000 ended sessions (oldest removed first). Removing a
  computer from the account also removes its sessions from the history.
- **API:** `GET /api/v2/account/history?limit=&cursor=&deviceIds=&direction=`
  (account-scoped, newest first, cursor pagination; `direction` is `any`,
  `incoming` or `outgoing` relative to `deviceIds`).
