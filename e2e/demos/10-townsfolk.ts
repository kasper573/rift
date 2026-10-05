import { test, type Page } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import {
  closestTile,
  hoverTile,
  hoverUi,
  occupied,
  travelTo,
  waitFor,
  waitUntilStill,
  type Snapshot,
  type Tile,
} from "../helpers/game";
import { fixture, townsperson } from "../helpers/talk";

test(
  "Townsfolk",
  chapter({
    summary: "Who lives on the island, and what they have for you",
    play: async (page) => {
      await caption(page, "Seven townsfolk live around the harbour — name plates show who they are");
      await walkBelow(page, (snapshot) => townsperson(snapshot, "Mara")?.at);
      await page.waitForTimeout(1500);
      await caption(page, "Hover a townsperson and a card lists everything they have for you");
      for (const npc of ["Mara", "Grisha", "Ilsa"]) {
        await hoverNpc(page, npc);
        await page.waitForTimeout(1800);
      }
      await caption(page, "An icon over a head is the most important thing they have for you — hover it");
      await hoverUi(page, "icons/attention/innkeeper.png");
      await page.waitForTimeout(2500);
      await caption(page, "Things on the map can be used too: a notice board, a chest by the tide");
      await walkBelow(page, (snapshot) => fixture(snapshot, "TideChest")?.at);
      const chest = await waitFor(page, (snapshot) => fixture(snapshot, "TideChest"), "no tide chest");
      await hoverTile(page, chest.aim);
      await page.waitForTimeout(2200);
      const board = await waitFor(page, (snapshot) => fixture(snapshot, "HarbourNotices"), "no notice board");
      await walkBelow(page, () => board.at);
      await hoverTile(page, board.aim);
      await page.waitForTimeout(2500);
    },
  }),
);

async function hoverNpc(page: Page, npc: string): Promise<void> {
  const body = await waitFor(page, (snapshot) => townsperson(snapshot, npc), `${npc} is not around`);
  await hoverTile(page, body.aim);
}

// Clear of the target itself, so the click walks there instead of interacting.
async function walkBelow(page: Page, target: (snapshot: Snapshot) => Tile | undefined): Promise<void> {
  await travelTo(page, (snapshot) => {
    const at = target(snapshot);
    const open = snapshot.walkable.filter((tile) => !occupied(snapshot, tile));
    return at && closestTile(open, [at[0], at[1] + 2.5]);
  });
  await waitUntilStill(page);
}
