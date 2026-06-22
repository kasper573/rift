import { expect, test } from "@playwright/test";

import { captureScene, clickFromCenter, register, waitForWorld } from "./helpers/game";
import { loadReference, resemblance } from "./helpers/image";

const MAP_MATCH = 0.5;

// The game draws every tile at 48 CSS px (render.rs), and a fresh player spawns centered three tiles
// south of the island warp — so the warp sits this far straight up from the canvas center.
const TILE_PX = 48;
const PORTAL_NORTH_PX = 3 * TILE_PX;

test("clicking the island portal crosses to the forest", async ({ page }) => {
  await register(page);
  await waitForWorld(page);

  const island = loadReference("island.png");
  const forest = loadReference("forest.png");

  const spawn = await captureScene(page);
  expect(resemblance(spawn, island), "the player should start on the island").toBeGreaterThan(
    resemblance(spawn, forest),
  );

  // One click on the warp tile: clicking inside a portal rect makes the client send MoveToPortal, so
  // the server walks the player onto the warp and crosses to the forest. (A plain walk over the rect
  // does not cross, so we must hit the tile — and must not re-click, which would cancel the cross.)
  await clickFromCenter(page, 0, -PORTAL_NORTH_PX);

  await expect
    .poll(
      async () => {
        const scene = await captureScene(page);
        const onForest = resemblance(scene, forest);
        return onForest >= MAP_MATCH && onForest > resemblance(scene, island);
      },
      { message: "clicking the portal should cross into the forest", timeout: 60_000, intervals: [1000] },
    )
    .toBe(true);
});
