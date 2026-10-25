"""Draws the island: a large landmass with coves in a wide sea, land guaranteed under everything
the scene places and the roads between them, meadow patches over the grass, and a landscape of
trees, rocks and bushes placed on the land away from the town. Writes the area's hard paint (grass,
meadow, water) and the Landscape folder of its scene; `tools/terrain.py` then gives it relief and
shores. Run as `python3 -I tools/island.py example_game/world/areas/island/island.area`."""
import json
import os
import sys

import numpy as np
from PIL import Image

SAMPLES_PER_METER = 2
CENTRE = (50.0, 50.0)
RADIUS = 36.0
COVES = [(0.7, 0.32, 15.0), (2.6, 0.28, 12.0), (-0.3, 0.22, 9.0)]
TOWN = (24.0, 13.0, 56.0, 36.0)
MEADOWS = [(62.0, 42.0, 6.5), (45.0, 63.0, 5.5), (72.0, 66.0, 7.0), (29.0, 56.0, 5.0)]
TREES = {"oak": 5, "oak-tall": 2, "pine": 3}
ROCKS = {"rock-small": 2, "rock-small-b": 2, "rock-large": 1, "rock-tall": 1, "rock-flat": 1, "rock-wide": 1}
BUSHES = {"bush": 2, "bush-small": 2, "bush-large": 1}
SEED = 20261025


def coast(columns, rows):
    y, x = (np.mgrid[0:rows, 0:columns] + 0.5) / SAMPLES_PER_METER
    dx, dy = x - CENTRE[0], y - CENTRE[1]
    angle = np.arctan2(dy, dx)
    reach = RADIUS + 5.0 * np.sin(3 * angle + 0.8) + 3.5 * np.sin(5 * angle + 2.1) + 2.0 * np.sin(9 * angle + 4.0)
    for at, width, depth in COVES:
        apart = np.angle(np.exp(1j * (angle - at)))
        reach = reach - depth * np.exp(-((apart / width) ** 2))
    return np.hypot(dx, dy) < reach, x, y


def placed_points(scene, config):
    points = [(config["initial_spawn"]["x"], config["initial_spawn"]["y"])] if "initial_spawn" in config else []

    def walk(nodes):
        for node in nodes:
            if node["type"] == "folder":
                walk(node.get("children", []))
            elif "at" in node:
                points.append((node["at"]["x"], node["at"]["y"]))
            elif node["type"] == "object":
                shape = node["shape"]
                points.append((shape["x"], shape["y"]))

    walk(scene)
    return points


def guaranteed(land, x, y, points):
    for px, py in points:
        land |= np.hypot(x - px, y - py) < 5.0
    for (ax, ay), (bx, by) in zip(points, points[1:]):
        t = np.clip(((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / max((bx - ax) ** 2 + (by - ay) ** 2, 1e-6), 0.0, 1.0)
        land |= np.hypot(x - (ax + t * (bx - ax)), y - (ay + t * (by - ay))) < 2.5
    return land


def shore_distance(land):
    far = np.full(land.shape, np.inf)
    for shift in range(1, 25):
        ring = np.zeros_like(land)
        for axis in (0, 1):
            for sign in (1, -1):
                ring |= np.roll(~land, sign * shift, axis=axis)
        far = np.where(np.isinf(far) & ring, shift / SAMPLES_PER_METER, far)
    return np.where(land, far, 0.0)


def paint(land, x, y):
    weights = np.zeros(land.shape + (4,), dtype=np.float32)
    weights[..., 3] = ~land
    meadow = np.zeros(land.shape, dtype=np.float32)
    for mx, my, radius in MEADOWS:
        meadow = np.maximum(meadow, 0.5 * np.clip(1.5 - np.hypot(x - mx, y - my) / radius, 0.0, 1.0))
    weights[..., 2] = land * meadow
    weights[..., 0] = land * (1.0 - meadow)
    return np.round(weights * 255.0).astype(np.uint8)


def in_town(px, py):
    return TOWN[0] <= px <= TOWN[2] and TOWN[1] <= py <= TOWN[3]


def landscape(land, x, y, rng):
    inland = shore_distance(land)
    rows, columns = land.shape
    props = []

    def pick(kinds):
        names = list(kinds)
        return rng.choice(names, p=np.array([kinds[n] for n in names], dtype=float) / sum(kinds.values()))

    def place(name, px, py, lo, hi):
        props.append({"type": "prop", "prop": f"world/props/{name}.prop", "at": {"x": round(px * 2) / 2, "y": round(py * 2) / 2}, "yaw": float(rng.integers(0, 360)), "scale": round(float(rng.uniform(lo, hi)), 2)})

    for gy in np.arange(3.0, rows / SAMPLES_PER_METER - 3.0, 6.0):
        for gx in np.arange(3.0, columns / SAMPLES_PER_METER - 3.0, 6.0):
            px, py = gx + rng.uniform(-2.2, 2.2), gy + rng.uniform(-2.2, 2.2)
            column, row = int(px * SAMPLES_PER_METER), int(py * SAMPLES_PER_METER)
            if not land[row, column] or in_town(px, py):
                continue
            depth = inland[row, column]
            roll = rng.random()
            if depth < 1.0:
                continue
            if depth < 3.0:
                if roll < 0.25:
                    place(pick(ROCKS), px, py, 0.7, 1.1)
                continue
            if roll < 0.45:
                place(pick(TREES), px, py, 0.9, 1.25)
            elif roll < 0.62:
                place(pick(ROCKS), px, py, 0.8, 1.3)
            elif roll < 0.8:
                place(pick(BUSHES), px, py, 0.9, 1.2)
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
    size = config["terrain"]["size"]
    columns, rows = size["width"] * SAMPLES_PER_METER, size["height"] * SAMPLES_PER_METER
    land, x, y = coast(columns, rows)
    land = guaranteed(land, x, y, placed_points(scene, settings))
    Image.fromarray(paint(land, x, y), "RGBA").save(os.path.join(area, "terrain", "paint.png"))
    scene = [node for node in scene if not (node["type"] == "folder" and node.get("label") == "Landscape")]
    scene.append({"type": "folder", "label": "Landscape", "children": landscape(land, x, y, np.random.default_rng(SEED))})
    with open(scene_file, "w") as file:
        json.dump(scene, file, indent=2)
        file.write("\n")
    print(f"{area}: {int(land.mean() * 100)}% land, {len(scene[-1]['children'])} props")


if __name__ == "__main__":
    main()
