import { expect, test } from "@playwright/test";

import { captureScene, clickFromPlayer, register, waitForWorld } from "./helpers/game";
import { diffFraction } from "./helpers/image";

// A short walk sweeps high-contrast scenery across the frame. Measured at a high per-pixel threshold
// so animated-water shimmer — which only nudges colors — stays far below it, leaving a wide margin
// between an idle frame and a walked one.
const MIN_DELTA = 100;
const WALKED = 0.15;

test("clicking the map visibly walks the player", async ({ page }) => {
  await register(page);
  await waitForWorld(page);

  const before = await captureScene(page);
  // Two tiles north is open beach, short of the warp three tiles up — the player walks without
  // crossing, and the camera scroll repaints the frame.
  await clickFromPlayer(page, 0, 2);

  await expect
    .poll(async () => diffFraction(before, await captureScene(page), MIN_DELTA), {
      message: "clicking the map should visibly walk the player",
      timeout: 30_000,
      intervals: [500],
    })
    .toBeGreaterThan(WALKED);
});
