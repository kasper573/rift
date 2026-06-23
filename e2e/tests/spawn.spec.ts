import { expect, test } from "@playwright/test";

import { captureScene, MAP_MATCH, register, waitForWorld } from "../helpers/game";
import { loadReference, resemblance } from "../helpers/image";

test("a new player spawns into the island scene", async ({ page }) => {
  await register(page);
  const island = loadReference("island.png");
  const forest = loadReference("forest.png");
  await waitForWorld(page, island);

  const scene = await captureScene(page);
  const onIsland = resemblance(scene, island);
  const onForest = resemblance(scene, forest);

  expect(
    onIsland,
    `spawn should resemble the island (island ${onIsland.toFixed(3)}, forest ${onForest.toFixed(3)})`,
  ).toBeGreaterThanOrEqual(MAP_MATCH);
  expect(onIsland).toBeGreaterThan(onForest);
});
