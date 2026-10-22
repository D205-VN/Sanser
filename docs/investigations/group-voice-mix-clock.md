# Group voice: staggered speakers multiplied playback blocks

Following the report that microphone audio stutters when two or three people
join while sharing, a device-free regression reproduced a defect in the mixer
shipped in 2.2.19. This demonstrates a code defect; it does not establish every
cause of lag on the user's physical network and audio devices.

Each speaker previously advanced its own 5 ms PCM deadline. `Mixer::drain(now)`
repeatedly called every speaker at the same wall time. When speaker deadlines had
different phases, one call could emit a partial mix for one speaker, followed by
another partial mix for a different speaker in the same output interval. One
48 kHz output device cannot play these extra blocks without dropping audio or
increasing its queue. This is separate from encoding once and forwarding to
multiple recipients; increasing the number of forwarded packets is expected.

## Reproduction

Use independent real Opus encoders with 20 ms packets, first-packet offsets of
0/6/13 ms, and a simulated worker waking every 7 ms. Receive and drain until
2002 ms. Run the identical fixture against the actual `v2.2.19` source and the
repaired source, using the same Opus library and PCM. No sockets, microphones or
speakers are needed for this comparison.

| Remote talkers | 2.2.19 output 5 ms blocks | Repaired output 5 ms blocks |
| --- | ---: | ---: |
| 1 | 397 | 397 |
| 2 | 509 | 397 |
| 3 | 565 | 397 |

The added regression fails on the released mixer and passes with the common
output clock. It also checks all staggered sources continue decoding without
stale drops. These are virtual-time PCM scheduling counts, not measured WAN RTT,
physical speaker latency or acoustic echo reduction.

## Repair and scope

`Mixer` retains a decoder, sequence, epoch, independent jitter reserve, FEC/PLC
and replay rejection for each speaker, but mixes once per shared output tick.
A late worker may drain elapsed ticks up to the existing eight-block budget.
A stall beyond 40 ms resynchronizes the output clock instead of playing a long
output backlog. Idle/clear resets that clock. This does not change microphone
consent, the codec, wire format, voice forwarding permissions or video settings.

The group-voice test also retains encrypted three-viewer forwarding, no own-voice
return, deafen and compatibility checks. Native panel tests retain no capture on
construction/expand, listen-only Join, explicit Unmute, collapse without leaving,
compact Join/Mute, role-specific Stop and sharing without voice. They use a fake
controller, never a real microphone. Real two/three-computer calls with shared
music and speakers still need a listening test after the new code is packaged.
