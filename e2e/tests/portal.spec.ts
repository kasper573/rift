import { expect, test } from "@playwright/test";

import { captureScene, clickUntil, register, waitForWorld } from "./helpers/game";
import { loadReference, resemblance } from "./helpers/image";

const MAP_MATCH = 0.5;

test("clicking the island portal crosses to the forest", async ({ page }) => {
  await register(page);
  await waitForWorld(page);

  const island = loadReference("island.png");
  const forest = loadReference("forest.png");

  const spawn = await captureScene(page);
  expect(resemblance(spawn, island), "the player should start on the island").toBeGreaterThan(
    resemblance(spawn, forest),
  );

  // The warp sits three tiles due north of the spawn. Clicking inside its rect makes the client send
  // MoveToPortal, so the server walks the player onto the warp and crosses to the forest.
  await clickUntil(
    page,
    spawn,
    0,
    3,
    (scene) => resemblance(scene, forest) >= MAP_MATCH && resemblance(scene, forest) > resemblance(scene, island),
    { message: "clicking the portal should cross into the forest", timeout: 45_000 },
  );
});
