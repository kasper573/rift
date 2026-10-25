"""Relief and shores for every area of a game whose paint came from hard cells: puddles too small
to be water become the area's third material, the ground sinks below the water level where water
is painted and rises inland into gentle hills, the paint blends across the shoreline, and the area's
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


def repaint(weights, signed):
    water = 1.0 - smoothstep(-0.75, 0.75, signed)
    shore = (1.0 - water) * (1.0 - smoothstep(1.0, 2.5, signed))
    rest = 1.0 - water - shore
    land = np.stack([blur(weights[..., i]) for i in range(3)], axis=-1)
    total = land.sum(axis=-1, keepdims=True)
    land = np.where(total > 0.0, land / np.maximum(total, 1e-6), [1.0, 0.0, 0.0])
    painted = land * rest[..., None]
    painted[..., 1] += shore
    return np.concatenate([painted, water[..., None]], axis=-1)


def to_bytes(painted):
    scaled = painted * 255.0
    bytes_ = np.floor(scaled).astype(np.int32)
    short = 255 - bytes_.sum(axis=-1)
    top = np.argmax(scaled - bytes_, axis=-1)
    np.put_along_axis(bytes_, top[..., None], np.take_along_axis(bytes_, top[..., None], -1) + short[..., None], -1)
    return np.clip(bytes_, 0, 255).astype(np.uint8)


def relief(area):
    folder = os.path.join(area, "terrain")
    paint_file = os.path.join(folder, "paint.png")
    with open(os.path.join(area, "config.json")) as config:
        sunlit = json.load(config).get("lighting", {}).get("sun") is not None
    if not sunlit or not os.path.exists(paint_file):
        return False
    weights = np.asarray(Image.open(paint_file).convert("RGBA"), dtype=np.float32) / 255.0
    rows, columns = weights.shape[:2]
    small = puddles(weights[..., 3] > 0.5)
    weights[small] = [0.0, 0.0, 1.0, 0.0]
    signed = shore_distance(blur(blur(weights[..., 3])) > 0.5)
    Image.fromarray(to_bytes(repaint(weights, signed)), "RGBA").save(paint_file)
    corner = np.pad(signed, ((0, 1), (0, 1)), mode="edge")
    field = heights(corner, columns + 1, rows + 1)
    level = np.clip((field - LOWEST) / (HIGHEST - LOWEST), 0.0, 1.0)
    Image.fromarray(np.round(level * 65535.0).astype(np.uint16)).save(os.path.join(folder, "height.png"))
    return True


def main():
    game = sys.argv[1]
    areas = sys.argv[2:] or sorted(glob.glob(os.path.join(game, "**", "*.area"), recursive=True))
    for area in areas:
        if relief(area):
            print("relief", area)


if __name__ == "__main__":
    main()
