import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { caption, chapter, enterWorld } from "../helpers/demo";
import { clickUi, closestTile, findUi, probe, waitFor, waitForWorld, walkTo } from "../helpers/game";
import { loadReference } from "../helpers/image";

let player: Page;

test(
  "Spectate",
  chapter({
    summary: "Watch the world live without taking part",
    setup: async (page, cast) => {
      player = await cast.newPage();
      await enterWorld(player);
      await signIn(page, await provisionAccount(page, ["spectator"]));
      await waitFor(page, (snapshot) => findUi(snapshot, "Spectate"), "the mode choice never showed");
    },
    play: async (page) => {
      await caption(page, "Spectators choose between playing and watching");
      await page.waitForTimeout(1500);
      await clickUi(page, "Spectate");
      await waitForWorld(page, loadReference("island.png"));
      await caption(page, "They see everyone in the area, while nobody sees them");
      for (const [dx, dy] of [
        [3, 1],
        [-2, 2],
        [-4, -1],
      ]) {
        const { me, walkable } = await probe(player);
        await walkTo(player, closestTile(walkable, [me!.at[0] + dx, me!.at[1] + dy])!);
      }
      await page.waitForTimeout(1000);
    },
  }),
);
