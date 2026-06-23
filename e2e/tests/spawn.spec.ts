import { expect, test } from "@playwright/test";

import { captureScene, register, waitForWorld } from "./helpers/game";
import { loadReference, resemblance } from "./helpers/image";

// "Basically the same place" as a reference. The matching map scores ~0.83 and the other ~0.03, so
// 0.5 is a wide margin; the island > forest comparison is the real, never-close discriminator.
const MAP_MATCH = 0.5;

test("a new player spawns into the island scene", async ({ page }) => {
  await register(page);
  await waitForWorld(page);

  const scene = await captureScene(page);
  const onIsland = resemblance(scene, loadReference("island.png"));
  const onForest = resemblance(scene, loadReference("forest.png"));

  expect(
    onIsland,
    `spawn should resemble the island (island ${onIsland.toFixed(3)}, forest ${onForest.toFixed(3)})`,
  ).toBeGreaterThanOrEqual(MAP_MATCH);
  expect(onIsland).toBeGreaterThan(onForest);
});
