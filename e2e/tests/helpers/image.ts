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

// What fraction of an 8×8 grid of cells shows scenery — a cell counts when it holds more than a
// handful of distinct colors. A rendered map lights up most cells; a blank or still-loading canvas
// almost none. Used to wait until the world is actually on screen, independent of which map it is.
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

// The center half of a frame, where the map fills the view — crops away the corner HUD and fps
// counter so they don't inflate the scenery measurement.
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

// Fraction of pixels whose color changed appreciably between two same-sized frames. A camera scroll
// (the player walking) repaints most of the frame; idle shimmer barely moves it.
export function diffFraction(a: Image, b: Image): number {
  if (a.width !== b.width || a.height !== b.height) {
    throw new Error(`frame size mismatch: ${a.width}x${a.height} vs ${b.width}x${b.height}`);
  }
  const pixels = a.width * a.height;
  let changed = 0;
  for (let p = 0; p < pixels; p++) {
    const i = 4 * p;
    const delta =
      Math.abs(a.data[i] - b.data[i]) +
      Math.abs(a.data[i + 1] - b.data[i + 1]) +
      Math.abs(a.data[i + 2] - b.data[i + 2]);
    if (delta > 24) changed++;
  }
  return changed / pixels;
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
      const r = bin(data[i]);
      const g = bin(data[i + 1]);
      const b = bin(data[i + 2]);
      hist[(r * HIST_BINS + g) * HIST_BINS + b] += 1;
    }
  }
  const total = HIST_SAMPLES * HIST_SAMPLES;
  for (let i = 0; i < hist.length; i++) hist[i] /= total;
  return hist;
}

// Color-histogram intersection in [0, 1]: 1 is an identical color distribution, 0 no overlap.
// Coarsely samples both frames so it answers "is this the same place" — tolerant of the player's
// exact position, animation, resolution, and a browser's rendering quirks rather than demanding
// exact pixels. This is what lets a single reference snapshot match across every browser project.
export function resemblance(a: Image, b: Image): number {
  const ha = histogram(a);
  const hb = histogram(b);
  let sum = 0;
  for (let i = 0; i < ha.length; i++) sum += Math.min(ha[i], hb[i]);
  return sum;
}
