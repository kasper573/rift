import { readFileSync } from "node:fs";
import { join } from "node:path";

import { PNG } from "pngjs";

// An RGBA frame in pngjs's native layout (4 bytes per pixel, row-major).
export interface Image {
  width: number;
  height: number;
  data: Buffer;
}

export function decode(png: Buffer): Image {
  const { width, height, data } = PNG.sync.read(png);
  return { width, height, data };
}

export function loadReference(name: string): Image {
  return decode(readFileSync(join(__dirname, "..", "..", "snapshots", name)));
}

const GRID = 8;
const CELL_COLORS = 4;

// Fraction of an 8×8 grid of cells holding scenery (a cell counts when it has more than a few distinct
// colors). A rendered map lights up most cells; a blank or still-loading canvas almost none.
export function sceneFraction(image: Image): number {
  const cellW = Math.floor(image.width / GRID);
  const cellH = Math.floor(image.height / GRID);
  if (cellW === 0 || cellH === 0) return 0;
  let busy = 0;
  for (let gy = 0; gy < GRID; gy++) {
    for (let gx = 0; gx < GRID; gx++) {
      const colors = new Set<number>();
      for (let y = gy * cellH; y < (gy + 1) * cellH; y++) {
        for (let x = gx * cellW; x < (gx + 1) * cellW; x++) {
          const i = 4 * (y * image.width + x);
          colors.add((image.data[i] << 16) | (image.data[i + 1] << 8) | image.data[i + 2]);
        }
      }
      if (colors.size > CELL_COLORS) busy++;
    }
  }
  return busy / (GRID * GRID);
}

// The center half of a frame, cropping the corner HUD and fps counter out of the scenery measure.
export function center(image: Image): Image {
  const width = Math.floor(image.width / 2);
  const height = Math.floor(image.height / 2);
  const left = Math.floor(image.width / 4);
  const top = Math.floor(image.height / 4);
  const data = Buffer.alloc(4 * width * height);
  for (let y = 0; y < height; y++) {
    const from = 4 * ((top + y) * image.width + left);
    image.data.copy(data, 4 * y * width, from, from + 4 * width);
  }
  return { width, height, data };
}

// A real health bar is ~150–320 px; require clearly more than a stray green pixel or two.
const MIN_MARKER_PIXELS = 20;

// Centroid of the bright-green health-bar pixels — only the local player draws one (render.rs
// `healthbar`), so this pinpoints your own character. Null when no solid bar is visible (callers
// treat that as "not ready" and retry).
export function greenMarker(image: Image): { x: number; y: number } | null {
  const { width, height, data } = image;
  let sx = 0;
  let sy = 0;
  let n = 0;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const i = 4 * (y * width + x);
      if (data[i + 1] > 180 && data[i] < 120 && data[i + 2] < 120) {
        sx += x;
        sy += y;
        n++;
      }
    }
  }
  return n >= MIN_MARKER_PIXELS ? { x: sx / n, y: sy / n } : null;
}

const HIST_BINS = 8;
const HIST_SAMPLES = 64;

function histogram(image: Image): Float64Array {
  const hist = new Float64Array(HIST_BINS * HIST_BINS * HIST_BINS);
  const { width, height, data } = image;
  if (width === 0 || height === 0) return hist;
  const bin = (c: number) => Math.min(Math.floor((c * HIST_BINS) / 256), HIST_BINS - 1);
  for (let ty = 0; ty < HIST_SAMPLES; ty++) {
    for (let tx = 0; tx < HIST_SAMPLES; tx++) {
      const sx = Math.min(Math.floor((tx * width) / HIST_SAMPLES), width - 1);
      const sy = Math.min(Math.floor((ty * height) / HIST_SAMPLES), height - 1);
      const i = 4 * (sy * width + sx);
      hist[(bin(data[i]) * HIST_BINS + bin(data[i + 1])) * HIST_BINS + bin(data[i + 2])] += 1;
    }
  }
  const total = HIST_SAMPLES * HIST_SAMPLES;
  for (let i = 0; i < hist.length; i++) hist[i] /= total;
  return hist;
}

// Color-histogram intersection in [0, 1] (1 identical, 0 no overlap): answers "is this the same
// place" tolerant of the player's position, animation, resolution, and a browser's rendering quirks,
// so one reference snapshot matches across every browser.
export function resemblance(a: Image, b: Image): number {
  const ha = histogram(a);
  const hb = histogram(b);
  let sum = 0;
  for (let i = 0; i < ha.length; i++) sum += Math.min(ha[i], hb[i]);
  return sum;
}
