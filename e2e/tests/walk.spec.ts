import { expect, test } from "@playwright/test";

import { captureScene, holdClick, register, waitForWorld } from "./helpers/game";
import { diffFraction } from "./helpers/image";

// Clicking must repaint at least this fraction of the frame. A camera scroll (the player walking)
// clears it easily; idle animation stays well below it, so the threshold attributes the change to
// the click and nothing else.
const WALKED = 0.2;

test("clicking the map visibly walks the player", async ({ page }) => {
  await register(page);
  await waitForWorld(page);

  const before = await captureScene(page);

  // West of the island spawn is open beach. Clicking there walks the player and scrolls the camera;
  // mid-height keeps the click clear of the corner HUD. Re-click each pass until the walk lands.
  await expect(async () => {
    await holdClick(page, 0.25, 0.5);
    const moved = diffFraction(before, await captureScene(page));
    expect(moved, `clicking should visibly walk the player (${(moved * 100).toFixed(0)}% changed)`).toBeGreaterThan(
      WALKED,
    );
  }).toPass({ timeout: 30_000, intervals: [1000] });
});
