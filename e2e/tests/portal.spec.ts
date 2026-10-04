import { expect, test } from "@playwright/test";

import { register } from "../helpers/account";
import { captureScene, clickTile, MAP_MATCH, probe, waitForWorld } from "../helpers/game";
import { loadReference, resemblance } from "../helpers/image";

test("clicking the island warp crosses to the forest", async ({ page }) => {
  await register(page);
  const island = loadReference("island.png");
  const forest = loadReference("forest.png");
  await waitForWorld(page, island);
  const warp = (await probe(page)).portals.find((portal) => portal.to === "Forest");
  expect(warp, "the island has a warp to the forest").toBeDefined();
  // Re-click the warp until the forest renders. Repeats just re-issue the (deterministic) crossing;
  // the long timeout is only room for a slow renderer.
  await expect
    .poll(
      async () => {
        const scene = await captureScene(page);
        if (resemblance(scene, forest) >= MAP_MATCH) {
          return true;
        }
        await clickTile(page, warp!.at);
        return false;
      },
      { message: "clicking the warp should cross into the forest", timeout: 120_000, intervals: [1000] },
    )
    .toBe(true);
});
