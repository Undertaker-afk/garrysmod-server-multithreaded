"""Offline executable tests using a generated, non-proprietary BSP subset fixture."""
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile


def fixture():
    # One world-model leaf, six axial planes, one brush: [0,10] x [-5,5]^2.
    data = bytearray(1036)
    struct.pack_into("<4si", data, 0, b"VBSP", 20)
    struct.pack_into("<i", data, 1032, 42)

    def lump(index, content, version=0):
        struct.pack_into("<4i", data, 8 + 16*index, len(data), len(content), version, 0)
        data.extend(content)

    lump(0, b'{"classname" "worldspawn"}\0')
    planes = [(1, 0, 0, 10), (-1, 0, 0, 0), (0, 1, 0, 5),
              (0, -1, 0, 5), (0, 0, 1, 5), (0, 0, -1, 5)]
    lump(1, b"".join(struct.pack("<4fi", *p, 0) for p in planes))
    lump(19, b"".join(struct.pack("<Hhhh", i, -1, -1, 0) for i in range(6)))
    lump(18, struct.pack("<iii", 0, 6, 1))
    lump(17, b"\0\0")
    leaf = bytearray(32)
    struct.pack_into("<HH", leaf, 24, 0, 1)
    lump(10, leaf, version=1)
    lump(14, struct.pack("<9f3i", -100, -100, -100, 100, 100, 100, 0, 0, 0, -1, 0, 0))
    return data


def main():
    binary = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix="scprust-bsp-") as directory:
        path = Path(directory) / "synthetic.bsp"
        path.write_bytes(fixture())

        def run(*args, status=0):
            completed = subprocess.run([binary, *map(str, args)], capture_output=True, text=True, timeout=10)
            assert completed.returncode == status, (completed.returncode, completed.stderr)
            if status:
                return completed
            return json.loads(completed.stdout)

        info = run("inspect", path)
        assert info["revision"] == 42 and info["world_brushes"] == 1
        assert info["entities"] == 1 and info["collision_scope"] == "world_brushes_only"
        hit = run("ray", path, "0x1", -10, 0, 0, 20, 0, 0)
        assert abs(hit["fraction"] - 1/3) < 1e-12
        assert hit["normal"] == [-1, 0, 0] and hit["brush"] == 0
        assert not hit["source_parity"]
        hull = run("hull", path, 1, -10, 0, 0, 20, 0, 0, -1, -1, -1, 1, 1, 1)
        assert abs(hull["fraction"] - 0.3) < 1e-12
        assert run("ray", path, 0, -10, 0, 0, 20, 0, 0)["brush"] is None
        run("ray", path, 1, "NaN", 0, 0, 1, 1, 1, status=2)
        bad = fixture()
        struct.pack_into("<i", bad, 4, 21)
        path.write_bytes(bad)
        assert "version 20" in run("inspect", path, status=1).stderr
        path.write_bytes(b"VBSP")
        run("inspect", path, status=1)
    print("Offline map inspector/collision smoke tests passed")


if __name__ == "__main__":
    main()
