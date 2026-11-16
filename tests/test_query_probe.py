"""Synthetic wire fixtures only; no proprietary packets or server identifiers."""
import struct
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import query_probe as probe


def fixture(edf=b"\0", extra=b""):
    return (probe.HEADER + b"I\x11test\0gm_test\0garrysmod\0Garry's Mod\0"
            + struct.pack("<H", 4000) + b"\x01\x96\0dl\0\x01test-version\0" + edf + extra)


class ProbeTests(unittest.TestCase):
    def test_minimal_response_and_optional_edf(self):
        result = probe.parse_info(fixture())
        self.assertEqual(result["app_id"], 4000)
        self.assertEqual(result["max_players"], 150)
        self.assertTrue(result["vac_enabled"])
        self.assertEqual(probe.parse_info(fixture()[:-1]), result)

    def test_all_extra_fields_follow_wire_order(self):
        extra = (struct.pack("<HQH", 27015, 1234, 27020) + b"spectator\0scp,test\0"
                 + struct.pack("<Q", 4000))
        result = probe.parse_info(fixture(b"\xf1", extra))
        self.assertEqual(result["port"], 27015)
        self.assertEqual(result["steam_id"], 1234)
        self.assertEqual(result["spectator_name"], "spectator")
        self.assertEqual(result["keywords"], "scp,test")
        self.assertEqual(result["game_id"], 4000)

    def test_truncated_and_unknown_packets_fail_closed(self):
        packet = fixture(b"\x80", struct.pack("<H", 27015))
        # EOF before EDF is legal; EDF declaring a missing field is not.
        for length in range(len(fixture()) - 1):
            with self.assertRaises(probe.ProtocolError):
                probe.parse_info(packet[:length])
        for invalid in (packet[:-1], fixture(b"\x02"), fixture() + b"junk",
                        b"\xfe\xff\xff\xff" + packet, b"x" * 4097):
            with self.assertRaises(probe.ProtocolError):
                probe.parse_info(invalid)

    def test_zero_challenge_is_echoed_as_opaque_bytes(self):
        with patch.object(probe.socket, "getaddrinfo", return_value=[(2, 2, 17, "", ("127.0.0.1", 27015))]), \
                patch.object(probe.socket, "socket") as factory:
            sock = factory.return_value.__enter__.return_value
            sock.recv.side_effect = [probe.HEADER + b"A\0\0\0\0", fixture()]
            result, raw = probe.query_info("127.0.0.1", 27015)
            self.assertEqual(raw, fixture())
            self.assertEqual(result["app_id"], 4000)
            self.assertEqual(sock.send.call_args_list[1].args[0], probe.INFO + b"\0" * 4)

    def test_challenge_retries_are_bounded(self):
        with patch.object(probe.socket, "getaddrinfo", return_value=[(2, 2, 17, "", ("127.0.0.1", 27015))]), \
                patch.object(probe.socket, "socket") as factory:
            sock = factory.return_value.__enter__.return_value
            sock.recv.return_value = probe.HEADER + b"A\x01\x02\x03\x04"
            with self.assertRaises(probe.ProtocolError):
                probe.query_info("127.0.0.1", 27015)
            self.assertEqual(sock.send.call_count, 3)

    def test_invalid_query_configuration_does_not_send(self):
        for port, timeout in [(0, 1), (65536, 1), (27015, float("nan")), (27015, 11)]:
            with self.assertRaises(ValueError):
                probe.query_info("127.0.0.1", port, timeout)


if __name__ == "__main__":
    unittest.main()
