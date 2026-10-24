"""Placeholder card art for every item of a game: the item's icon, scaled up, over a parchment frame
tinted per item, 256 x 180 pixels. Run as `python3 -I tools/cards.py example_game`; a card that is
already there is left alone, so a painted card is never overwritten."""
import glob
import hashlib
import json
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from glb import read_png, write_png  # noqa: E402

WIDTH, HEIGHT = 256, 180
PARCHMENT = np.array([214, 190, 140], dtype=np.float32)
INK = np.array([70, 50, 30], dtype=np.float32)


def card(icon, seed):
    rng = np.random.default_rng(int.from_bytes(hashlib.sha256(seed.encode()).digest()[:8], "little"))
    hue = rng.uniform(0.0, 1.0)
    tint = np.array([np.sin(hue * 6.283), np.sin((hue + 0.33) * 6.283), np.sin((hue + 0.66) * 6.283)]) * 22.0
    y, x = np.mgrid[0:HEIGHT, 0:WIDTH].astype(np.float32)
    grain = rng.normal(0.0, 4.0, (HEIGHT, WIDTH, 1))
    vignette = (((x - WIDTH / 2) / (WIDTH / 2)) ** 2 + ((y - HEIGHT / 2) / (HEIGHT / 2)) ** 2)[:, :, None]
    pixels = PARCHMENT + tint - vignette * 55.0 + grain
    border = (x < 6) | (x >= WIDTH - 6) | (y < 6) | (y >= HEIGHT - 6)
    inner = ((x >= 6) & (x < 9)) | ((x >= WIDTH - 9) & (x < WIDTH - 6)) | ((y >= 6) & (y < 9)) | ((y >= HEIGHT - 9) & (y < HEIGHT - 6))
    pixels[border] = INK
    pixels[inner] = INK * 0.5 + PARCHMENT * 0.5
    scale = 4
    big = np.repeat(np.repeat(icon, scale, axis=0), scale, axis=1)
    h, w = big.shape[:2]
    top, left = (HEIGHT - h) // 2, (WIDTH - w) // 2
    alpha = big[:, :, 3:4].astype(np.float32) / 255.0
    region = pixels[top : top + h, left : left + w]
    shadow = np.roll(np.roll(alpha, 3, axis=0), 3, axis=1)
    region[:] = region * (1.0 - shadow * 0.35)
    region[:] = region * (1.0 - alpha) + big[:, :, :3].astype(np.float32) * alpha
    out = np.clip(pixels, 0, 255).astype(np.uint8)
    return np.concatenate([out, np.full((HEIGHT, WIDTH, 1), 255, dtype=np.uint8)], axis=2)


def main(game):
    for path in sorted(glob.glob(f"{game}/**/*.item.json", recursive=True)):
        item = json.load(open(path))
        if "card" in item and os.path.exists(f"{game}/{item['card']}"):
            continue
        name = os.path.basename(path).split(".")[0]
        icon = read_png(f"{game}/{item['icon']}")
        file = f"assets/cards/{name}.png"
        write_png(f"{game}/{file}", card(icon, name))
        item["card"] = file
        json.dump(item, open(path, "w"), indent=2, ensure_ascii=False)
        open(path, "a").write("\n")
        print(file)


if __name__ == "__main__":
    main(sys.argv[1])
