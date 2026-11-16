"""Offline executable tests using a generated, non-proprietary BSP subset fixture."""
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile


def box_planes(mins, maxs):
    return [
        (1, 0, 0, maxs[0]),
        (-1, 0, 0, -mins[0]),
        (0, 1, 0, maxs[1]),
        (0, -1, 0, -mins[1]),
        (0, 0, 1, maxs[2]),
        (0, 0, -1, -mins[2]),
    ]


def fixture():
    # World model (leaf 0): floor [-200,200]^2 x [-64,0] and box [40,50] x [-5,5] x [0,10].
    # Submodel *1 (leaf 1): [-5,5]^2 x [0,20], placed at origin (100,0,0) yaw 90.
    data = bytearray(1036)
    struct.pack_into("<4si", data, 0, b"VBSP", 20)
    struct.pack_into("<i", data, 1032, 42)

    def lump(index, content, version=0):
        struct.pack_into("<4i", data, 8 + 16 * index, len(data), len(content), version, 0)
        data.extend(content)

    entities = (
        b'{"classname" "worldspawn"}\n'
        b'{"classname" "func_brush" "model" "*1" "origin" "100 0 0" "angles" "0 90 0"}\n'
        b'{"classname" "info_player_start" "origin" "0 0 0" "angles" "0 90 0"}\n'
        b'{"classname" "info_player_deathmatch" "origin" "500 0 0"}\0'
    )
    lump(0, entities)
    planes = (
        box_planes((-200, -200, -64), (200, 200, 0))
        + box_planes((40, -5, 0), (50, 5, 10))
        + box_planes((-5, -5, 0), (5, 5, 20))
    )
    lump(1, b"".join(struct.pack("<4fi", *p, 0) for p in planes))
    lump(19, b"".join(struct.pack("<Hhhh", i, -1, -1, 0) for i in range(len(planes))))
    lump(
        18,
        b"".join(
            struct.pack("<iii", first, 6, 1) for first in (0, 6, 12)
        ),
    )
    lump(17, struct.pack("<3H", 0, 1, 2))
    leaves = bytearray(64)
    struct.pack_into("<HH", leaves, 24, 0, 2)
    struct.pack_into("<HH", leaves, 32 + 24, 2, 1)
    lump(10, leaves, version=1)
    models = b"".join(
        struct.pack("<9f3i", -1000, -1000, -1000, 1000, 1000, 1000, 0, 0, 0, head, 0, 0)
        for head in (-1, -2)
    )
    lump(14, models)
    return data


def main():
    binary = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix="scprust-bsp-") as directory:
        path = Path(directory) / "synthetic.bsp"
        path.write_bytes(fixture())

        def run(*args, status=0):
            completed = subprocess.run(
                [binary, *map(str, args)], capture_output=True, text=True, timeout=10
            )
            assert completed.returncode == status, (completed.returncode, completed.stderr)
            if status:
                return completed
            return json.loads(completed.stdout)

        info = run("inspect", path)
        assert info["revision"] == 42 and info["world_brushes"] == 2
        assert info["entities"] == 4 and info["solid_models"] == 1
        assert info["spawn_points"] == 2 and info["valid_spawns"] == 1
        assert info["collision_scope"] == "world_and_static_brush_models"

        models = run("models", path)
        assert len(models["models"]) == 1
        assert models["models"][0]["name"] == "*1"
        assert models["models"][0]["origin"] == [100.0, 0.0, 0.0]

        spawns = run("spawns", path)
        assert len(spawns["spawns"]) == 2
        by_class = {s["classname"]: s for s in spawns["spawns"]}
        assert by_class["info_player_start"]["valid"] is True
        assert by_class["info_player_start"]["on_ground"] is True
        assert by_class["info_player_deathmatch"]["valid"] is False

        hit = run("ray", path, "0x1", 20, 0, 5, 60, 0, 5)
        assert abs(hit["fraction"] - 0.5) < 1e-12
        assert hit["normal"] == [-1, 0, 0] and hit["brush"] == 1
        assert hit["collision_scope"] == "world_and_static_brush_models"
        assert not hit["source_parity"]

        hull = run("hull", path, 1, 20, 0, 5, 60, 0, 5, -1, -1, -1, 1, 1, 1)
        assert abs(hull["fraction"] - 0.475) < 1e-12

        miss_world = run("ray", path, 1, 80, 0, 5, 120, 0, 5, "--world")
        assert miss_world["brush"] is None
        assert miss_world["collision_scope"] == "world_brushes_only"

        hit_solids = run("ray", path, 1, 80, 0, 5, 120, 0, 5)
        assert abs(hit_solids["fraction"] - 0.375) < 1e-12
        assert hit_solids["brush"] == 2

        hit_model = run("ray", path, 1, 80, 0, 5, 120, 0, 5, "--model", 1)
        assert abs(hit_model["fraction"] - 0.375) < 1e-12
        assert hit_model["collision_scope"] == "single_brush_model"

        assert run("ray", path, 0, 20, 0, 5, 60, 0, 5)["brush"] is None
        run("ray", path, 1, "NaN", 0, 0, 1, 1, 1, status=2)
        run("ray", path, 1, 0, 0, 0, 1, 1, 1, "--model", 99, status=1)

        bad = fixture()
        struct.pack_into("<i", bad, 4, 21)
        path.write_bytes(bad)
        assert "version 20" in run("inspect", path, status=1).stderr
        path.write_bytes(b"VBSP")
        run("inspect", path, status=1)
    print("Offline map inspector/collision smoke tests passed")


if __name__ == "__main__":
    main()
