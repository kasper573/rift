"""Relief and shores for every area of a game whose paint came from hard cells: puddles too small
to be water become stone, the ground sinks below the water level where water is painted and rises
inland into gentle hills, a band of sand (or dirt) runs along the shoreline, and the area's
second material is laid as a band of shore along the water.
Run as `python3 -I tools/terrain.py example_game`; every area under the sun with a paint part is
rewritten, and interiors keep their flat floors. Name an area folder after the game to rewrite
that one alone. Relief is drawn from hard paint and does not repeat on its own output."""
import glob
import json
import os
import sys

import numpy as np
from PIL import Image

SAMPLES_PER_METER = 2
LOWEST, HIGHEST = -8.0, 16.0
WATER_LEVEL = 0.0
FAR = 24.0
PUDDLE_PIXELS = 48


def smoothstep(edge0, edge1, x):
    t = np.clip((x - edge0) / (edge1 - edge0), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


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
    """Metres from every pixel to the nearest pixel of `inside`."""
    far = float((FAR * SAMPLES_PER_METER) ** 2) * 4.0
    squared = np.where(inside, 0.0, far)
    squared = np.apply_along_axis(distance_1d, 1, squared)
    squared = np.apply_along_axis(distance_1d, 0, squared)
    return np.minimum(np.sqrt(squared), FAR * SAMPLES_PER_METER) / SAMPLES_PER_METER


def shore_distance(water):
    """Signed metres from the shoreline: positive on land, negative in the water."""
    if not water.any():
        return np.full(water.shape, FAR)
    half = 0.5 / SAMPLES_PER_METER
    return np.where(water, -(distance(~water) - half), distance(water) - half)


def blur(channel):
    out = channel.copy()
    for axis in (0, 1):
        out = (np.roll(out, 1, axis=axis) + out + np.roll(out, -1, axis=axis)) / 3.0
    return out


def puddles(water):
    """The water pixels in bodies of fewer than PUDDLE_PIXELS pixels."""
    rows, columns = water.shape
    seen = np.zeros_like(water)
    small = np.zeros_like(water)
    for start in zip(*np.nonzero(water)):
        if seen[start]:
            continue
        body = [start]
        seen[start] = True
        stack = [start]
        while stack:
            y, x = stack.pop()
            for ny, nx in ((y - 1, x), (y + 1, x), (y, x - 1), (y, x + 1)):
                if 0 <= ny < rows and 0 <= nx < columns and water[ny, nx] and not seen[ny, nx]:
                    seen[ny, nx] = True
                    body.append((ny, nx))
                    stack.append((ny, nx))
        if len(body) < PUDDLE_PIXELS:
            for pixel in body:
                small[pixel] = True
    return small


def hills(columns, rows):
    y, x = np.mgrid[0:rows, 0:columns] / SAMPLES_PER_METER
    rolling = 0.5 + 0.5 * np.sin(x / 9.0 + 1.3) * np.sin(y / 11.0 + 0.4)
    swell = 0.5 + 0.5 * np.sin((x + y) / 17.0)
    return 0.6 * rolling + 0.4 * swell


def heights(signed, columns, rows):
    land = 0.5 * smoothstep(0.0, 3.0, signed) + hills(columns, rows) * smoothstep(2.0, 12.0, signed)
    water = -1.6 * smoothstep(0.0, 4.0, -signed)
    return WATER_LEVEL + np.where(signed >= 0.0, land, water)


def repaint(index, signed, shore_index, water_index):
    """Water where the ground sinks, a band of the shore terrain along it, the land's paint beyond."""
    water = signed < 0.0
    shore = ~water & (signed < 2.5) if shore_index is not None else np.zeros_like(water)
    land = index.copy()
    land[water] = water_index
    land[shore] = shore_index
    return land


def terrain_index(terrains, name):
    """The index of the terrain file called `name`, if the area lists one."""
    for i, path in enumerate(terrains):
        if os.path.basename(path) == f"{name}.terrain.json":
            return i
    return None


def relief(area):
    paint_file = os.path.join(area, "paint.png")
    with open(os.path.join(area, "config.json")) as file:
        config = json.load(file)
    sunlit = config.get("lighting", {}).get("sun") is not None
    if not sunlit or not os.path.exists(paint_file):
        return False
    terrains = config["terrains"]
    water_index = terrain_index(terrains, "water")
    if water_index is None:
        return False
    shore_index = terrain_index(terrains, "sand")
    if shore_index is None:
        shore_index = terrain_index(terrains, "dirt")
    puddle_index = terrain_index(terrains, "stone")
    index = np.array(Image.open(paint_file).convert("L"), dtype=np.uint8)
    rows, columns = index.shape
    water = index == water_index
    small = puddles(water)
    if puddle_index is not None:
        index[small] = puddle_index
    else:
        index[small] = 0
    signed = shore_distance(blur(blur((index == water_index).astype(np.float32))) > 0.5)
    Image.fromarray(repaint(index, signed, shore_index, water_index), "L").save(paint_file)
    corner = np.pad(signed, ((0, 1), (0, 1)), mode="edge")
    field = heights(corner, columns + 1, rows + 1)
    level = np.clip((field - LOWEST) / (HIGHEST - LOWEST), 0.0, 1.0)
    Image.fromarray(np.round(level * 65535.0).astype(np.uint16)).save(os.path.join(area, "height.png"))
    return True


def main():
    game = sys.argv[1]
    areas = sys.argv[2:] or sorted(glob.glob(os.path.join(game, "**", "*.area"), recursive=True))
    for area in areas:
        if relief(area):
            print("relief", area)


if __name__ == "__main__":
    main()
