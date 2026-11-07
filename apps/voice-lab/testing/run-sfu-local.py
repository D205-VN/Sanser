#!/usr/bin/env python3
"""Explicit isolated SFU check; synthetic Opus, never a microphone or real DB."""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--postgres-bin", required=True, type=Path)
    parser.add_argument("--livekit-server", required=True, type=Path)
    parser.add_argument("--peer-driver", required=True, type=Path)
    parser.add_argument("--target-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--sample-seconds", default=10, type=int,
                        help="synthetic three-peer sample duration, 10–600 seconds")
    parser.add_argument("--require-production-revocation", action="store_true",
                        help="fail if a cached token can rejoin after room closure")
    args = parser.parse_args()
    if not 10 <= args.sample_seconds <= 600:
        parser.error("--sample-seconds must be between 10 and 600")
    root = Path(__file__).resolve().parents[3]
    for port in (55439, 17880, 17881, 17883):
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", port))
    args.output.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(args.target_dir.resolve()))
    env["SANSER_LIVEKIT_TEST_SAMPLE_SECONDS"] = str(args.sample_seconds)
    env["TEST_DATABASE_URL"] = (
        "postgresql://sanser_test:isolated-local-test-password@127.0.0.1:55439/postgres?sslmode=disable"
    )
    pg_ctl = str(args.postgres_bin.resolve() / "pg_ctl")
    with tempfile.TemporaryDirectory(prefix="sanser-voice-check-", dir="/private/tmp" if Path("/private/tmp").is_dir() else None) as work:
        work = Path(work)
        password = work / "password"
        password.write_text("isolated-local-test-password\n")
        password.chmod(0o600)
        data, sockets = work / "data", work / "sockets"
        sockets.mkdir()
        subprocess.run([
            str(args.postgres_bin.resolve() / "initdb"), "-D", str(data),
            "--username=sanser_test", "--pwfile=" + str(password),
            "--encoding=UTF8", "--locale=C", "--auth-host=scram-sha-256", "--auth-local=trust",
        ], check=True, stdout=subprocess.DEVNULL)
        pg_started = False
        livekit = peer = None
        with (args.output / "livekit-runtime.log").open("w") as sfu_log, \
                (args.output / "peer-runtime.log").open("w") as peer_log:
            try:
                subprocess.run([
                    pg_ctl, "-D", str(data), "-l", str((args.output / "postgres.log").resolve()),
                    "-o", f"-h 127.0.0.1 -p 55439 -k {sockets}", "-w", "start",
                ], check=True, stdout=subprocess.DEVNULL)
                pg_started = True
                config = work / "livekit.yaml"
                config.write_text(
                    "port: 17880\nbind_addresses: [127.0.0.1]\nrtc:\n"
                    "  tcp_port: 17881\n  udp_port: 17882\n  use_external_ip: false\n"
                    "  enable_loopback_candidate: true\nkeys:\n  test_key: " + "a" * 32 +
                    "\nlogging:\n  level: warn\n"
                )
                config.chmod(0o600)
                livekit = subprocess.Popen([
                    str(args.livekit_server.resolve()), "--bind", "127.0.0.1",
                    "--node-ip", "127.0.0.1", "--config", str(config),
                ], stdout=sfu_log, stderr=subprocess.STDOUT)
                import time
                for _ in range(100):
                    if livekit.poll() is not None:
                        raise RuntimeError("local SFU exited before readiness")
                    try:
                        with socket.create_connection(("127.0.0.1", 17880), timeout=0.2):
                            break
                    except OSError:
                        time.sleep(0.1)
                else:
                    raise RuntimeError("local SFU readiness deadline")
                cargo = shutil.which("cargo")
                if not cargo:
                    raise RuntimeError("cargo is required")
                base = [cargo, "test", "--locked", "--offline", "-p", "sanser-server"]
                with (args.output / "postgres-api.log").open("w") as log:
                    subprocess.run(base + ["--test", "api", "--", "--ignored"], cwd=root,
                                   env=env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=600)
                exchange = work / "exchange"
                exchange.mkdir(mode=0o700)
                tone = work / "synthetic-tone.ogg"
                subprocess.run([
                    "ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
                    f"sine=frequency=440:sample_rate=48000:duration={args.sample_seconds + 20}", "-ac", "1", "-c:a",
                    "libopus", "-b:a", "24k", "-frame_duration", "20", str(tone),
                ], check=True)
                # Compile before starting the peer's bounded credential wait.
                with (args.output / "sfu-test-compile.log").open("w") as log:
                    subprocess.run(base + ["--test", "voice_sfu", "--no-run"], cwd=root,
                                   env=env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=600)
                peer = subprocess.Popen([str(args.peer_driver.resolve()), str(exchange)],
                                        stdout=peer_log, stderr=subprocess.STDOUT)
                env.update(SANSER_LIVEKIT_TEST_EXCHANGE_DIR=str(exchange),
                           SANSER_LIVEKIT_TEST_TONE=str(tone))
                with (args.output / "real-sfu.log").open("w") as log:
                    subprocess.run(base + ["--test", "voice_sfu", "--", "--ignored", "--nocapture"],
                                   cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT,
                                   check=True, timeout=args.sample_seconds + 120)
                peer.wait(timeout=15)
                if peer.returncode:
                    raise RuntimeError("synthetic peer driver failed")
                report = json.loads((exchange / "result.json").read_text())
                report.update(gatewayRevocationVerified=not report["cachedTokenRejoinSucceeded"] and not report["refreshedTokenRejoinSucceeded"], hardwareAudioVerified=False)
                (args.output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
                print(json.dumps(report))
                if args.require_production_revocation and not report["gatewayRevocationVerified"]:
                    raise RuntimeError("release gate failed: cached SFU token still admits a participant")
            finally:
                # Preserve only the peer's generic error/numeric reports when
                # diagnosing a failure; credentials never leave the temp dir.
                for name in ("error.json", "ready.json"):
                    report_file=work / "exchange" / name
                    if report_file.exists():
                        shutil.copyfile(report_file,args.output / name)
                # Stop only the processes/cluster created by this invocation.
                for process in (peer, livekit):
                    if process is not None and process.poll() is None:
                        process.terminate()
                        try:
                            process.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait()
                if pg_started:
                    subprocess.run([pg_ctl, "-D", str(data), "-m", "fast", "-w", "stop"],
                                   check=True, stdout=subprocess.DEVNULL)


if __name__ == "__main__":
    main()
