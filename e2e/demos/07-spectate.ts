import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { caption, chapter, enterWorld } from "../helpers/demo";
import {
  clickUi,
  closestTile,
  findUi,
  focusGame,
  nearest,
  probe,
  travelTo,
  waitFor,
  walkTo,
  type Tile,
} from "../helpers/game";

let islander: Page;
let explorer: Page;

test(
  "Spectate",
  chapter({
    summary: "Follow any player live, wherever they are",
    setup: async (page, cast) => {
      islander = await cast.newPage();
      await enterWorld(islander);
      explorer = await cast.newPage();
      await enterWorld(explorer);
      const { me, portals } = await probe(explorer);
      const warp = nearest(portals, me!.at)!;
      await travelTo(explorer, warp.at);
      await waitFor(explorer, ({ area }) => area === warp.to, `never arrived in ${warp.to}`);
      await signIn(page, await provisionAccount(page, ["spectator"]));
      await waitFor(page, (snapshot) => findUi(snapshot, "Spectate"), "the mode choice never showed");
    },
    play: async (page, cast) => {
      const islanderName = (await probe(islander)).me!.name;
      const explorerName = (await probe(explorer)).me!.name;
      await caption(page, "Spectators choose between playing and watching");
      await page.waitForTimeout(1500);
      await clickUi(page, "Spectate");
      await switchTo(page, islanderName);
      await caption(page, "A spectator always follows a player, and sees what they see");
      await stroll(islander, [
        [3, 1],
        [-2, 2],
      ]);
      await caption(page, "The arrow keys switch to the next player…");
      await switchTo(page, explorerName);
      await caption(page, "…wherever in the world they are");
      await stroll(explorer, [
        [2, 2],
        [-3, 1],
      ]);
      await caption(page, "The buttons on the spectator panel switch too");
      await clickUi(page, "<");
      await watching(page, islanderName);
      await page.waitForTimeout(2000);
      await caption(page, "When the watched player leaves, the spectator moves on to the next");
      await islander.close();
      await watching(page, explorerName);
      await page.waitForTimeout(3000);
      await caption(page, "And with nobody playing, there is nothing to watch");
      await explorer.close();
      await waitFor(
        page,
        (snapshot) => findUi(snapshot, "Nobody is playing right now"),
        "the nobody-playing screen never showed",
      );
      await page.waitForTimeout(2500);
      await caption(page, "…until someone joins, whom the spectator then follows");
      const newcomer = await cast.newPage();
      await enterWorld(newcomer);
      await watching(page, (await probe(newcomer)).me!.name);
      await stroll(newcomer, [[-3, 1]]);
      await page.waitForTimeout(1500);
    },
  }),
);

// Players left over from elsewhere may be online too, so step until the wanted one is on screen.
async function switchTo(page: Page, name: string): Promise<void> {
  await waitFor(page, ({ viewpoint }) => viewpoint, "the spectator never followed anyone");
  for (let step = 0; (await probe(page)).viewpoint?.name !== name; step++) {
    if (step > 10) throw new Error(`never got to watch ${name}`);
    await focusGame(page);
    await page.keyboard.press("ArrowRight");
    await page.waitForTimeout(1000);
  }
  await watching(page, name);
}

async function watching(page: Page, name: string): Promise<void> {
  await waitFor(page, ({ viewpoint }) => viewpoint?.name === name, `never watched ${name}`);
}

async function stroll(player: Page, steps: Tile[]): Promise<void> {
  for (const [dx, dy] of steps) {
    const { me, walkable } = await probe(player);
    await walkTo(player, closestTile(walkable, [me!.at[0] + dx, me!.at[1] + dy])!);
  }
}
