import { expect, test } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { admin } from "../helpers/admin";
import { captureScene, clickUi, leaveConversation, MAP_MATCH, probe, waitFor, waitForWorld } from "../helpers/game";
import { loadReference, resemblance } from "../helpers/image";
import { travel } from "./travel";

test("the forest road holds you without a pass and crosses with one", async ({ page }) => {
  await signIn(page, await provisionAccount(page, ["admin"]));
  await clickUi(page, "Play");
  const island = loadReference("island.png");
  const forest = loadReference("forest.png");
  await waitForWorld(page, island);
  const road = (await probe(page)).portals.find((portal) => portal.name === "forest-road");
  expect(road, "the island has a forest road").toBeDefined();

  await travel(page, road!.at);
  const held = await waitFor(page, ({ stage }) => stage, "Ilsa never halted you on the road");
  expect(held.node).toBe("ilsa_halt");
  expect((await probe(page)).area).toBe("island");
  await leaveConversation(page);

  await admin(page, [["/give road_pass,1", /gave 1 Road Pass/]]);
  // Re-click the warp until the forest renders. Repeats just re-issue the (deterministic) crossing;
  // the long timeout is only room for a slow renderer.
  await expect
    .poll(
      async () => {
        const scene = await captureScene(page);
        if (resemblance(scene, forest) >= MAP_MATCH) {
          return true;
        }
        if ((await probe(page)).area === "island") await travel(page, road!.at);
        return false;
      },
      { message: "with a pass, clicking the warp should cross into the forest", timeout: 120_000, intervals: [1000] },
    )
    .toBe(true);
});
