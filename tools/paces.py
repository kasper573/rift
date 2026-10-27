"""Measures how fast each clip of a library carries a body over the ground: the speed a planted
foot slides back under it, which is the `paces` entry a skeleton states for the clip.

    python3 tools/paces.py LIBRARY.glb FOOT[,FOOT...] [CLIP...]

FOOT names the joints that touch the ground. A foot counts as planted while it is within a fifth of
its lift of the lowest it gets in the clip; the pace is how far the planted feet move along the
ground over how long they stay planted, in metres a second at the library's own scale.
"""

import sys

import numpy as np

import glb

SAMPLES_PER_SECOND = 240
PLANTED = 0.2


def main():
    path, feet = sys.argv[1], sys.argv[2].split(",")
    only = set(sys.argv[3:])
    library = glb.Gltf(path)
    nodes = library.json["nodes"]
    parents = {child: index for index, node in enumerate(nodes) for child in node.get("children", [])}
    named = {node.get("name"): index for index, node in enumerate(nodes)}
    missing = [foot for foot in feet if foot not in named]
    if missing:
        sys.exit(f"no joint named {', '.join(missing)} in {path}")
    for animation in library.json.get("animations", []):
        name = animation.get("name", "")
        if only and name not in only:
            continue
        tracks, length = channels(library, animation)
        times = np.arange(0.0, length, 1.0 / SAMPLES_PER_SECOND)
        speeds = []
        for foot in feet if len(times) > 1 else []:
            path_of = np.array([placed(nodes, parents, tracks, named[foot], t) for t in times])
            speeds.extend(planted_speeds(path_of))
        pace = float(np.mean(speeds)) if speeds else 0.0
        print(f"{name:24s} {pace:5.2f} m/s   ({length:.2f} s)")


def channels(library, animation):
    tracks = {}
    length = 0.0
    for channel in animation["channels"]:
        sampler = animation["samplers"][channel["sampler"]]
        times = library.accessor(sampler["input"])[:, 0]
        values = library.accessor(sampler["output"])
        tracks[(channel["target"]["node"], channel["target"]["path"])] = (times, values)
        length = max(length, float(times[-1]))
    return tracks, length


def planted_speeds(points):
    heights = points[:, 1]
    low, high = heights.min(), heights.max()
    planted = heights <= low + PLANTED * max(high - low, 1e-6)
    steps = np.diff(points[:, [0, 2]], axis=0) * SAMPLES_PER_SECOND
    speeds = np.linalg.norm(steps, axis=1)
    return list(speeds[planted[:-1] & planted[1:]])


def placed(nodes, parents, tracks, joint, time):
    chain = []
    while joint is not None:
        chain.append(joint)
        joint = parents.get(joint)
    matrix = np.eye(4)
    for index in reversed(chain):
        node = nodes[index]
        translation = np.array(node.get("translation", [0, 0, 0]), float)
        rotation = np.array(node.get("rotation", [0, 0, 0, 1]), float)
        scale = np.array(node.get("scale", [1, 1, 1]), float)
        if (index, "translation") in tracks:
            translation = sample(*tracks[(index, "translation")], time, False)
        if (index, "rotation") in tracks:
            rotation = sample(*tracks[(index, "rotation")], time, True)
        if (index, "scale") in tracks:
            scale = sample(*tracks[(index, "scale")], time, False)
        local = np.eye(4)
        local[:3, :3] = turn(rotation) * scale
        local[:3, 3] = translation
        matrix = matrix @ local
    return matrix[:3, 3]


def sample(times, values, time, rotation):
    if time <= times[0]:
        return values[0]
    if time >= times[-1]:
        return values[-1]
    index = int(np.searchsorted(times, time)) - 1
    blend = (time - times[index]) / (times[index + 1] - times[index])
    after = values[index + 1]
    if rotation and np.dot(values[index], after) < 0:
        after = -after
    value = values[index] * (1 - blend) + after * blend
    return value / np.linalg.norm(value) if rotation else value


def turn(quaternion):
    x, y, z, w = quaternion
    return np.array(
        [
            [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
            [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
            [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
        ]
    )


if __name__ == "__main__":
    main()
