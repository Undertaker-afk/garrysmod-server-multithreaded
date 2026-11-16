#!/usr/bin/env python3
"""Query ONE authorized Source endpoint; no scanning, sign-on, or credentials.

Keeps the raw info datagram for interoperability fixtures. Review output before
sharing: names, maps, IPs and optional server Steam IDs can identify your server.
"""
import argparse
import datetime
import json
import math
import socket
import struct
import sys

HEADER = b"\xff\xff\xff\xff"
INFO = HEADER + b"TSource Engine Query\0"
MAX_PACKET = 4096


class ProtocolError(ValueError):
    pass


class Reader:
    def __init__(self, data):
        self.data = data
        self.offset = 0

    def read(self, length):
        if length > len(self.data) - self.offset:
            raise ProtocolError("truncated info response")
        result = self.data[self.offset:self.offset + length]
        self.offset += length
        return result

    def number(self, fmt):
        return struct.unpack("<" + fmt, self.read(struct.calcsize("<" + fmt)))[0]

    def string(self):
        end = self.data.find(b"\0", self.offset)
        if end < 0 or end - self.offset > 1024:
            raise ProtocolError("missing/oversized NUL-terminated string")
        value = self.read(end - self.offset + 1)[:-1]
        return value.decode("utf-8", errors="replace")


def parse_info(packet):
    if len(packet) > MAX_PACKET:
        raise ProtocolError("oversized info packet")
    if packet.startswith(b"\xfe\xff\xff\xff"):
        raise ProtocolError("split/compressed responses are not supported by this probe")
    reader = Reader(packet)
    if reader.read(5) != HEADER + b"I":
        raise ProtocolError("not a modern single-packet A2S_INFO response")
    info = {"protocol": reader.number("B"), "server_name": reader.string(),
            "map": reader.string(), "folder": reader.string(), "game": reader.string(),
            "app_id": reader.number("H"), "players": reader.number("B"),
            "max_players": reader.number("B"), "bots": reader.number("B"),
            "server_type": reader.read(1).decode("ascii", errors="replace"),
            "platform": reader.read(1).decode("ascii", errors="replace")}
    for name in ("password_protected", "vac_enabled"):
        value = reader.number("B")
        if value not in (0, 1):
            raise ProtocolError("invalid boolean field")
        info[name] = bool(value)
    info["version"] = reader.string()
    edf = reader.number("B") if reader.offset < len(packet) else 0
    if edf & ~0xF1:
        raise ProtocolError("unknown extra-data flags")
    info["edf"] = edf
    if edf & 0x80:
        info["port"] = reader.number("H")
    if edf & 0x10:
        info["steam_id"] = reader.number("Q")
    if edf & 0x40:
        info["spectator_port"] = reader.number("H")
        info["spectator_name"] = reader.string()
    if edf & 0x20:
        info["keywords"] = reader.string()
    if edf & 0x01:
        info["game_id"] = reader.number("Q")
    if reader.offset != len(packet):
        raise ProtocolError("unparsed trailing bytes")
    return info


def query_info(host, port, timeout=1.0):
    if not 1 <= port <= 65535 or not math.isfinite(timeout) or not 0 < timeout <= 10:
        raise ValueError("port must be 1..65535 and timeout in (0, 10] seconds")
    family, kind, proto, _, address = socket.getaddrinfo(host, port, type=socket.SOCK_DGRAM)[0]
    with socket.socket(family, kind, proto) as sock:
        # A connected UDP socket rejects datagrams from unrelated source peers.
        sock.settimeout(timeout)
        sock.connect(address)
        challenge = None
        for _ in range(3):
            sock.send(INFO + (challenge if challenge is not None else b""))
            packet = sock.recv(MAX_PACKET + 1)
            if packet.startswith(HEADER + b"A"):
                if len(packet) != 9:
                    raise ProtocolError("invalid challenge response length")
                challenge = packet[5:9]  # Opaque four bytes, including zero.
                continue
            return parse_info(packet), packet
        raise ProtocolError("server kept issuing challenges; retry limit reached")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--timeout", type=float, default=1.0)
    parser.add_argument("--output", help="New JSON file; refuses to overwrite an existing file")
    args = parser.parse_args()
    try:
        info, raw = query_info(args.host, args.port, args.timeout)
        report = {"schema": 1, "kind": "source-a2s-info",
                  "recorded_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  "target": {"host": args.host, "port": args.port}, "info": info,
                  "raw_info_hex": raw.hex(),
                  "compatibility_note": "Discovery only; does not establish sign-on or gameplay support."}
        text = json.dumps(report, indent=2, ensure_ascii=True) + "\n"
        if args.output:
            with open(args.output, "x", encoding="utf-8") as output:
                output.write(text)
        else:
            print(text, end="")
    except (OSError, ValueError) as error:
        print(f"query failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
