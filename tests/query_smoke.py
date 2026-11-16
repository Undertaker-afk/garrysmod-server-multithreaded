"""Bounded real-UDP executable test. Terminates its child even on failure."""
import json
import os
import selectors
import socket
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import query_probe as probe


def exchange(sock, packet):
    sock.send(packet)
    return sock.recv(4096)


def main():
    binary = str(Path(sys.argv[1]).resolve())
    process = subprocess.Popen([binary, "--bind", "127.0.0.1:0", "--name", "smoke"],
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            if not selector.select(timeout=10):
                raise AssertionError("server did not start in 10 seconds")
        line = process.stdout.readline().strip()
        assert line.startswith("QUERY_ONLY_READY "), line
        host, port = line.split()[1].rsplit(":", 1)
        address = (host, int(port))
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(1)
            sock.connect(address)
            challenge = exchange(sock, probe.INFO)
            assert len(challenge) == 9 and challenge[:5] == probe.HEADER + b"A"
            token = challenge[5:]
            info = probe.parse_info(exchange(sock, probe.INFO + token))
            assert info["server_name"] == "[QUERY ONLY] smoke"
            assert info["app_id"] == 4000 and info["folder"] == "garrysmod"
            assert info["players"] == info["max_players"] == info["bots"] == 0
            assert not info["vac_enabled"] and info["map"] == "no_map_loaded"
            for code, expected in [(b"U", b"D\0"), (b"V", b"E\0\0")]:
                initial = exchange(sock, probe.HEADER + code + b"\xff" * 4)
                assert initial == challenge
                assert exchange(sock, probe.HEADER + code + token) == probe.HEADER + expected
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as other:
                other.settimeout(1)
                other.connect(address)
                assert exchange(other, probe.INFO + token)[:5] == probe.HEADER + b"A"
            sock.settimeout(0.05)
            for invalid in (b"", b"junk", probe.INFO + b"x", probe.INFO + b"x" * 6000,
                            probe.HEADER + b"getchallenge\0", b"\xfe\xff\xff\xffT"):
                sock.send(invalid)
                try:
                    unexpected = sock.recv(4096)
                except socket.timeout:
                    continue
                raise AssertionError(f"replied to unsupported packet: {unexpected!r}")
        parsed, raw = probe.query_info(*address)
        assert parsed == info and raw[:5] == probe.HEADER + b"I"
        if os.environ.get("SCPRUST_TEST_A2S") == "1":
            # Independent client, not our decoder; pinned test dependency in CI.
            import a2s
            external = a2s.info(address, timeout=1)
            assert external.server_name == info["server_name"]
            assert external.max_players == 0 and not external.vac_enabled
            assert a2s.players(address, timeout=1) == []
            assert a2s.rules(address, timeout=1) == {}
        with tempfile.TemporaryDirectory(prefix="scprust-query-report-") as directory:
            output = Path(directory) / "probe.local.json"
            cli = subprocess.run([sys.executable, str(Path(probe.__file__)), "--host", host,
                                  "--port", str(port), "--output", str(output)],
                                 capture_output=True, timeout=5)
            assert cli.returncode == 0, cli.stderr
            report = json.loads(output.read_text(encoding="utf-8"))
            assert report["info"] == info
            assert bytes.fromhex(report["raw_info_hex"]) == raw
        denied = subprocess.run([binary, "--bind", "0.0.0.0:0"], capture_output=True, timeout=5)
        assert denied.returncode == 2 and b"loopback" in denied.stderr
        print("Standalone UDP query smoke test passed")
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        process.stdout.close()
        process.stderr.close()


if __name__ == "__main__":
    main()
