import { expect, test } from "@playwright/test";

import { captureScene, clickWorldTile, MAP_MATCH, register, waitForWorld } from "../helpers/game";
import { loadReference, resemblance } from "../helpers/image";

// The island's warp tile to the forest (assets/maps/island.tmx warp #7); its rect is tiles
// x[38.5, 39.5] y[25.5, 26.5], so (39, 26) is squarely inside.
const WARP_TILE = { x: 39, y: 26 };

test("clicking the island warp crosses to the forest", async ({ page }) => {
  await register(page);
  const island = loadReference("island.png");
  const forest = loadReference("forest.png");
  await waitForWorld(page, island);

  const spawn = await captureScene(page);
  expect(resemblance(spawn, island), "the player should start on the island").toBeGreaterThan(
    resemblance(spawn, forest),
  );

  // Re-click the warp until the forest renders. The fixed-tile click is idempotent, so repeats just
  // re-issue the (deterministic) crossing; the long timeout is only room for a slow renderer.
  await expect
    .poll(
      async () => {
        const scene = await captureScene(page);
        if (resemblance(scene, forest) >= MAP_MATCH && resemblance(scene, forest) > resemblance(scene, island)) {
          return true;
        }
        await clickWorldTile(page, WARP_TILE.x, WARP_TILE.y);
        return false;
      },
      { message: "clicking the warp should cross into the forest", timeout: 120_000, intervals: [1000] },
    )
    .toBe(true);
});
