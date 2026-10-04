import { defineConfig } from "@playwright/test";

import { baseURL, chromiumSilence, hostResolver } from "./playwright.config";

// Software WebGL runs the game at a few frames per second, far from what players see. gl-egl renders
// on the GPU's own driver where there is one, and falls back to Mesa's software renderer otherwise.
const gpu = ["--use-gl=angle", "--use-angle=gl-egl", "--ignore-gpu-blocklist"];
// A player hears the world from their first click on the page; a chapter, often recording before its
// first click, would otherwise open in silence.
const autoplay = ["--autoplay-policy=no-user-gesture-required"];

export const VIDEO = { width: 1280, height: 720 };

export default defineConfig({
  testDir: "./demos",
  testMatch: "**/*.ts",
  // Every chapter plays on the same game server, so one at a time keeps each video to its own players.
  workers: 1,
  timeout: 300_000,
  reporter: [["list"]],
  use: {
    baseURL,
    ignoreHTTPSErrors: true,
    browserName: "chromium",
    channel: "chrome",
    launchOptions: {
      args: [...gpu, ...autoplay, "--no-sandbox", "--disable-dev-shm-usage", ...chromiumSilence, ...hostResolver],
    },
    viewport: VIDEO,
    deviceScaleFactor: 1,
    actionTimeout: 30_000,
    navigationTimeout: 60_000,
  },
});
