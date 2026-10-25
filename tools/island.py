"""Draws the island, the game's home area: a wide landmass with a harbour cove in the west, the
town clearing flat in the middle, the cottage on a rise to the north-east, a bone camp among rocks
to the south-west, the bat rocks to the south-east and dead trees along the north edge; dirt paths
between them, meadow patches, a sand shore and the sea beyond. Land is guaranteed under everything
the scene places and along the paths, so the scene stays valid. Writes the area's terrains, its
paint and height maps and the Landscape folder of its scene (trees, rocks, bushes and tufts on the
land away from the places), leaving every other folder to the designer.
Run as `python3 -I tools/island.py example_game/world/areas/island/island.area`."""
import json
import os
import sys

import numpy as np
from PIL import Image

SAMPLES_PER_METER = 2
CENTRE = (64.0, 64.0)
RADIUS = 46.0
COVES = [(np.pi, 0.28, 9.0), (-1.15, 0.22, 8.0), (1.75, 0.28, 11.0)]
BULGES = [(-np.pi / 2, 0.4, 7.0)]
CLEARING = (50.0, 50.0, 78.0, 72.0)
RISE = (88.0, 40.0, 10.0, 2.0)
BONE_CAMP = (36.0, 92.0, 7.0)
BAT_ROCKS = (92.0, 92.0, 9.0)
DEAD_TREES = (56.0, 18.0, 80.0, 32.0)
HARBOUR = (0.0, 56.0, 36.0, 72.0)
MEADOWS = [(80.0, 58.0, 7.0), (46.0, 40.0, 6.0), (100.0, 70.0, 7.0), (40.0, 52.0, 5.0), (70.0, 100.0, 6.0)]
PATHS = [
    [(64.0, 50.0), (64.0, 40.0), (68.0, 30.0)],
    [(66.0, 48.0), (76.0, 44.0), (87.0, 42.5)],
    [(50.0, 62.0), (40.0, 63.0), (32.0, 64.0)],
    [(54.0, 72.0), (44.0, 82.0), (36.0, 90.0)],
    [(76.0, 72.0), (84.0, 80.0), (90.0, 88.0)],
]
PATH_WIDTH = 1.8
SHORE_WIDTH = 2.5
WATER_LEVEL = 0.0
FAR = 24.0
TREES = {"oak": 5, "oak-tall": 2, "pine": 3}
ROCKS = {"rock-small": 2, "rock-small-b": 2, "rock-large": 1, "rock-tall": 1, "rock-flat": 1, "rock-wide": 1}
BUSHES = {"bush": 2, "bush-small": 2, "bush-large": 1}
TUFTS = {"tuft": 2, "tuft-large": 1, "tuft-leafy": 1}
SEED = 20261025
TERRAINS = [
    "world/terrain/grass.terrain.json",
    "world/terrain/meadow.terrain.json",
    "world/terrain/dirt.terrain.json",
    "world/terrain/stone.terrain.json",
    "world/terrain/sand.terrain.json",
    "world/terrain/water.terrain.json",
]
GRASS, MEADOW, DIRT, STONE, SAND, WATER = range(6)


def smoothstep(edge0, edge1, x):
    t = np.clip((x - edge0) / (edge1 - edge0), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


GROUND_ORIGIN = -0.5
LOWEST, HIGHEST = -8.0, 16.0


def pixel_grid(size):
    """The metres under the middle of every paint pixel."""
    y, x = (np.mgrid[0 : size["height"] * SAMPLES_PER_METER, 0 : size["width"] * SAMPLES_PER_METER] + 0.5) / SAMPLES_PER_METER
    return x + GROUND_ORIGIN, y + GROUND_ORIGIN


def corner_grid(size):
    """The metres under every height sample, one more per side than the pixels."""
    y, x = np.mgrid[0 : size["height"] * SAMPLES_PER_METER + 1, 0 : size["width"] * SAMPLES_PER_METER + 1] / SAMPLES_PER_METER
    return x + GROUND_ORIGIN, y + GROUND_ORIGIN


def coast(x, y):
    dx, dy = x - CENTRE[0], y - CENTRE[1]
    angle = np.arctan2(dy, dx)
    reach = RADIUS + 6.0 * np.sin(3 * angle + 0.8) + 4.0 * np.sin(5 * angle + 2.1) + 2.5 * np.sin(9 * angle + 4.0)
    for at, width, depth in COVES:
        apart = np.angle(np.exp(1j * (angle - at)))
        reach = reach - depth * np.exp(-((apart / width) ** 2))
    for at, width, rise in BULGES:
        apart = np.angle(np.exp(1j * (angle - at)))
        reach = reach + rise * np.exp(-((apart / width) ** 2))
    return np.hypot(dx, dy) < reach


def placed_points(scene, config):
    points = [(config["initial_spawn"]["x"], config["initial_spawn"]["y"])] if "initial_spawn" in config else []

    def walk(nodes):
        for node in nodes:
            if node["type"] == "folder":
                walk(node.get("children", []))
            elif "at" in node and isinstance(node["at"].get("x"), (int, float)):
                points.append((node["at"]["x"], node["at"]["y"]))
            elif node["type"] == "object":
                shape = node["shape"]
                points.append((shape["x"] + shape.get("w", 0.0) / 2, shape["y"] + shape.get("h", 0.0) / 2))
            elif node["type"] == "population" and "roams" in node:
                roams = node["roams"]
                points.append((roams["x"] + roams["w"] / 2, roams["y"] + roams["h"] / 2))

    walk(scene)
    return points


def segment_distance(x, y, a, b):
    (ax, ay), (bx, by) = a, b
    t = np.clip(((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / max((bx - ax) ** 2 + (by - ay) ** 2, 1e-6), 0.0, 1.0)
    return np.hypot(x - (ax + t * (bx - ax)), y - (ay + t * (by - ay)))


def guaranteed(land, x, y, points):
    for px, py in points:
        land |= np.hypot(x - px, y - py) < 8.0
    for path in PATHS:
        for a, b in zip(path, path[1:]):
            land |= segment_distance(x, y, a, b) < 3.0
    cx, cy, radius, _ = RISE
    land |= np.hypot(x - cx, y - cy) < radius + 4.0
    for cx, cy, radius in (BONE_CAMP, BAT_ROCKS):
        land |= np.hypot(x - cx, y - cy) < radius + 3.0
    return land


def distance_1d(f):
    """Squared distances along one line to its nearest cell of `f` 0, by lower parabola envelopes."""
    n = len(f)
    d = np.empty(n)
    v = np.zeros(n, dtype=int)
    z = np.empty(n + 1)
    z[0], z[1] = -np.inf, np.inf
    k = 0
    for q in range(1, n):
        s = ((f[q] + q * q) - (f[v[k]] + v[k] * v[k])) / (2 * q - 2 * v[k])
        while s <= z[k]:
            k -= 1
            s = ((f[q] + q * q) - (f[v[k]] + v[k] * v[k])) / (2 * q - 2 * v[k])
        k += 1
        v[k] = q
        z[k] = s
        z[k + 1] = np.inf
    k = 0
    for q in range(n):
        while z[k + 1] < q:
            k += 1
        d[q] = (q - v[k]) ** 2 + f[v[k]]
    return d


def distance(inside):
    """Metres from every sample to the nearest sample of `inside`."""
    far = float((FAR * SAMPLES_PER_METER) ** 2) * 4.0
    squared = np.where(inside, 0.0, far)
    squared = np.apply_along_axis(distance_1d, 1, squared)
    squared = np.apply_along_axis(distance_1d, 0, squared)
    return np.minimum(np.sqrt(squared), FAR * SAMPLES_PER_METER) / SAMPLES_PER_METER


def shore_distance(land):
    """Signed metres from the shoreline: positive on land, negative in the water."""
    half = 0.5 / SAMPLES_PER_METER
    return np.where(land, distance(~land) - half, -(distance(land) - half))


def box_weight(x, y, box, feather):
    """1 inside the box, falling to 0 over `feather` metres outside it."""
    x0, y0, x1, y1 = box
    outside = np.hypot(np.maximum(x0 - x, 0.0) + np.maximum(x - x1, 0.0), np.maximum(y0 - y, 0.0) + np.maximum(y - y1, 0.0))
    return 1.0 - smoothstep(0.0, feather, outside)


def heights(signed, x, y):
    rolling = 0.5 + 0.5 * np.sin(x / 9.0 + 1.3) * np.sin(y / 11.0 + 0.4)
    swell = 0.5 + 0.5 * np.sin((x + y) / 17.0)
    hills = (0.6 * rolling + 0.4 * swell) * smoothstep(2.0, 12.0, signed)
    hills *= 1.0 - box_weight(x, y, CLEARING, 8.0)
    cx, cy, radius, height = RISE
    rise = height * (1.0 - smoothstep(radius, radius + 5.0, np.hypot(x - cx, y - cy))) * smoothstep(0.0, 6.0, signed)
    land = 0.5 * smoothstep(0.0, 3.0, signed) + hills + rise
    water = -1.6 * smoothstep(0.0, 4.0, -signed)
    return WATER_LEVEL + np.where(signed >= 0.0, land, water)


def paint(signed, x, y):
    """The terrain index of every half metre."""
    index = np.full(x.shape, GRASS, dtype=np.uint8)
    meadow = np.zeros(x.shape, dtype=np.float32)
    for mx, my, radius in MEADOWS:
        meadow = np.maximum(meadow, np.clip(1.5 - np.hypot(x - mx, y - my) / radius, 0.0, 1.0))
    index[meadow > 0.75] = MEADOW
    for cx, cy, radius in (BAT_ROCKS,):
        index[np.hypot(x - cx, y - cy) < radius] = STONE
    for cx, cy, radius in (BONE_CAMP,):
        index[np.hypot(x - cx, y - cy) < radius] = DIRT
    for path in PATHS:
        for a, b in zip(path, path[1:]):
            index[segment_distance(x, y, a, b) < PATH_WIDTH / 2] = DIRT
    index[(signed >= 0.0) & (signed < SHORE_WIDTH)] = SAND
    index[signed < 0.0] = WATER
    return index


def in_box(px, py, box):
    return box[0] <= px <= box[2] and box[1] <= py <= box[3]


def kept_clear(px, py):
    cx, cy, radius, _ = RISE
    if np.hypot(px - cx, py - cy) < radius + 2.0:
        return True
    for cx, cy, radius in (BONE_CAMP, BAT_ROCKS):
        if np.hypot(px - cx, py - cy) < radius + 3.0:
            return True
    if in_box(px, py, CLEARING) or in_box(px, py, DEAD_TREES) or in_box(px, py, HARBOUR):
        return True
    return any(segment_distance(px, py, a, b) < 2.0 for path in PATHS for a, b in zip(path, path[1:]))


def landscape(signed, rng):
    rows, columns = signed.shape
    props = []

    def pick(kinds):
        names = list(kinds)
        return rng.choice(names, p=np.array([kinds[n] for n in names], dtype=float) / sum(kinds.values()))

    def place(name, px, py, lo, hi):
        props.append({"type": "prop", "prop": f"world/props/{name}.prop", "at": {"x": round(px * 2) / 2, "y": round(py * 2) / 2}, "yaw": float(rng.integers(0, 360)), "scale": round(float(rng.uniform(lo, hi)), 2)})

    for gy in np.arange(3.0, rows / SAMPLES_PER_METER - 4.0, 5.0):
        for gx in np.arange(3.0, columns / SAMPLES_PER_METER - 4.0, 5.0):
            px, py = gx + rng.uniform(-1.8, 1.8), gy + rng.uniform(-1.8, 1.8)
            depth = signed[int((py - GROUND_ORIGIN) * SAMPLES_PER_METER), int((px - GROUND_ORIGIN) * SAMPLES_PER_METER)]
            if depth < 1.0 or kept_clear(px, py):
                continue
            roll = rng.random()
            if depth < 3.0:
                if roll < 0.25:
                    place(pick(ROCKS), px, py, 0.7, 1.1)
                continue
            if roll < 0.45:
                place(pick(TREES), px, py, 0.9, 1.25)
            elif roll < 0.6:
                place(pick(ROCKS), px, py, 0.8, 1.3)
            elif roll < 0.78:
                place(pick(BUSHES), px, py, 0.9, 1.2)
            elif roll < 0.95:
                place(pick(TUFTS), px, py, 0.8, 1.3)
    return props


def main():
    area = sys.argv[1]
    with open(os.path.join(area, "config.json")) as file:
        config = json.load(file)
    scene_file = os.path.join(area, "scene.json")
    with open(scene_file) as file:
        scene = json.load(file)
    game = area[: area.index("/world/")] if "/world/" in area else os.path.dirname(area)
    with open(os.path.join(game, "fixtures", "game.config.json")) as file:
        settings = json.load(file)
    config["terrains"] = TERRAINS
    with open(os.path.join(area, "config.json"), "w") as file:
        json.dump(config, file, indent=2)
        file.write("\n")
    size = config["size"]
    points = placed_points(scene, settings)
    x, y = pixel_grid(size)
    land = guaranteed(coast(x, y), x, y, points)
    signed = shore_distance(land)
    Image.fromarray(paint(signed, x, y), "L").save(os.path.join(area, "paint.png"))
    cx, cy = corner_grid(size)
    corner_signed = shore_distance(guaranteed(coast(cx, cy), cx, cy, points))
    level = np.clip((heights(corner_signed, cx, cy) - LOWEST) / (HIGHEST - LOWEST), 0.0, 1.0)
    Image.fromarray(np.round(level * 65535.0).astype(np.uint16)).save(os.path.join(area, "height.png"))
    scene = [node for node in scene if not (node["type"] == "folder" and node.get("label") == "Landscape")]
    scene.append({"type": "folder", "label": "Landscape", "children": landscape(signed, np.random.default_rng(SEED))})
    with open(scene_file, "w") as file:
        json.dump(scene, file, indent=2)
        file.write("\n")
    print(f"{area}: {int(land.mean() * 100)}% land, {len(scene[-1]['children'])} props")


if __name__ == "__main__":
    main()
