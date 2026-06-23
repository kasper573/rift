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
  return decode(readFileSync(join(__dirname, "..", "snapshots", name)));
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
