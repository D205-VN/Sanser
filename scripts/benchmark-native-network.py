#!/usr/bin/env python3
"""Emulate network conditions on loopback; never connects to a remote device.
Measurements exercise new SNV2 Peer only, not legacy Mac->Windows, codecs, OS input,
Wi-Fi/NAT, or WSS relay. Delays/limits below are test scenarios, not real networks.
"""
import argparse
import heapq
import json
import platform
import random
import selectors
import socket
import subprocess
import tempfile
import time
from pathlib import Path

SCENARIOS = [
    dict(name="lan_emulated", delay_ms=1, jitter_ms=.25, loss=0, mbps=100),
    dict(name="wan_emulated", delay_ms=25, jitter_ms=8, loss=.005, mbps=40),
    dict(name="wan_congested_emulated", delay_ms=60, jitter_ms=20, loss=.02, mbps=15),
]

def run(binary, scenario, seconds, seed):
    rng = random.Random(seed)
    sockets = [socket.socket(socket.AF_INET, socket.SOCK_DGRAM) for _ in range(4)]
    for sock in sockets:
        sock.bind(("127.0.0.1", 0))
    ports = [sock.getsockname()[1] for sock in sockets]
    for sock in sockets[:2]:
        sock.close()
    selector = selectors.DefaultSelector()
    for index, sock in enumerate(sockets[2:]):
        sock.setblocking(False)
        selector.register(sock, selectors.EVENT_READ, index)
    queue = []
    serial = 0
    wire_free = [0.0, 0.0]
    traffic = dict(packets=0, bytes=0, random_drops=0, queue_drops=0, send_errors=0, peak_queued=0)
    began = time.monotonic()
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        process = subprocess.Popen([str(binary), *map(str, ports), str(seconds)], stdout=out, stderr=err)
        try:
            while process.poll() is None:
                now = time.monotonic()
                if now-began > seconds+15:
                    raise TimeoutError("Synthetic benchmark exceeded its deadline")
                while queue and queue[0][0] <= now:
                    _, _, direction, payload = heapq.heappop(queue)
                    try:
                        sockets[3-direction].sendto(payload, ("127.0.0.1", ports[1-direction]))
                    except (BlockingIOError, OSError):
                        traffic["send_errors"] += 1
                timeout = max(0, min(.005, queue[0][0]-now)) if queue else .005
                for key, _ in selector.select(timeout):
                    direction = key.data
                    # Bound each read batch so forwarding is never starved by video.
                    for _ in range(128):
                        try:
                            payload = key.fileobj.recv(65536)
                        except BlockingIOError:
                            break
                        traffic["packets"] += 1
                        traffic["bytes"] += len(payload)
                        if rng.random() < scenario["loss"]:
                            traffic["random_drops"] += 1
                            continue
                        now = time.monotonic()
                        serialization = len(payload)*8/(scenario["mbps"]*1_000_000)
                        ready = max(wire_free[direction], now)+serialization
                        if ready-now > .09:
                            traffic["queue_drops"] += 1
                            continue
                        wire_free[direction] = ready
                        delay = max(0, scenario["delay_ms"]+rng.uniform(-scenario["jitter_ms"], scenario["jitter_ms"]))/1000
                        serial += 1
                        heapq.heappush(queue, (ready+delay, serial, direction, payload))
                        traffic["peak_queued"] = max(traffic["peak_queued"], len(queue))
            out.seek(0)
            err.seek(0)
            result = json.loads(out.read().decode())
            stderr = err.read()
            return dict(scenario=scenario, seed=seed, seconds=seconds, result=result, traffic=traffic,
                        exit_code=process.returncode, stderr_bytes=len(stderr))
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            selector.close()
            for sock in sockets:
                sock.close()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--seconds", type=int, default=8)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not 1 <= args.seconds <= 60 or not 1 <= args.repeats <= 10:
        parser.error("seconds must be 1..60 and repeats must be 1..10")
    report = dict(scope="SIMULATED new SNV2 transport only; not Windows or real LAN/WAN", platform=platform.platform(),
                  payload_mbps=20, target_fps=60, mouse_hz=120, keyboard_events_per_second=20, runs=[])
    for index, scenario in enumerate(SCENARIOS):
        for repeat in range(args.repeats):
            result = run(args.binary.resolve(), scenario, args.seconds, 1000+index*100+repeat)
            report["runs"].append(result)
            print(json.dumps(result), flush=True)
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(report, indent=2)+"\n")

if __name__ == "__main__":
    main()
