import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { admin } from "../helpers/admin";
import { caption, chapter } from "../helpers/demo";
import {
  clickTile,
  clickUi,
  closestTile,
  focusGame,
  holding,
  hoverTile,
  hoverUi,
  occupied,
  probe,
  travelTo,
  waitFor,
  waitForWorld,
  waitUntilStill,
  type Snapshot,
  type Tile,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { fixture, townsperson } from "../helpers/talk";

const SWORD = "icons/weapon_and_tool/iron_sword.png";

test(
  "Townsfolk",
  chapter({
    summary: "Who lives on the island, and what they have for you",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await admin(page, [["/give RustySword,1", /gave 1 Rusty Sword/]]);
    },
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

      await caption(page, "Loot at a townsperson's feet? A click picks it up before it talks");
      const { walkable, actors } = await probe(page);
      const host = ["Mara", "Wren", "Bram", "Grisha"]
        .map((npc) => actors.find((actor) => actor.npc === npc))
        .find((actor) => actor && walkable.some((tile) => tile[0] === actor.at[0] && tile[1] === actor.at[1]))!;
      await caption(page, `(Admin shortcut: drop a sword right where ${host.name} stands)`, { at: "top" });
      await admin(page, [[`/tp ${host.at[0]},${host.at[1]}`, /teleported/]]);
      await focusGame(page);
      await page.keyboard.press("KeyI");
      await page.keyboard.down("Control");
      await clickUi(page, SWORD);
      await page.keyboard.up("Control");
      await waitFor(page, (snapshot) => holding(snapshot, "RustySword") === 0, "the sword never dropped");
      await focusGame(page);
      await page.keyboard.press("KeyI");
      await walkBelow(page, () => host.at);
      const sword = await waitFor(
        page,
        ({ items }) => items.find((item) => item.item === "RustySword"),
        "the sword is not on the ground",
      );
      const underFoot: Tile = [sword.at[0], sword.at[1] - 0.3];
      await hoverTile(page, underFoot);
      await page.waitForTimeout(2000);
      await clickTile(page, underFoot);
      await waitFor(
        page,
        (snapshot) => holding(snapshot, "RustySword") === 1 && !snapshot.stage,
        `${host.name} talked instead of letting you pick the sword up`,
      );
      await page.waitForTimeout(1500);
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
