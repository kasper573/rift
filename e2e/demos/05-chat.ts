import { test, type Page } from "@playwright/test";

import { caption, chapter, enterWorld } from "../helpers/demo";
import { clickUi, closestTile, focusGame, probe, walkTo } from "../helpers/game";

test(
  "Chat",
  chapter({
    summary: "Meet other players and talk in the terminal",
    play: async (page, cast) => {
      await caption(page, "Other players share the world in real time…");
      const friend = await cast.newPage();
      await enterWorld(friend);
      await caption(page, "…like this one, who just signed in");
      const { me, walkable } = await probe(friend);
      await walkTo(friend, closestTile(walkable, [me!.at[0] - 3, me!.at[1] - 1])!);
      await caption(page, "Press C for the terminal, where everyone can chat");
      await say(page, "hello! anyone around?");
      await page.waitForTimeout(1200);
      await say(friend, "hi! let's go hunt some orcs");
      await page.waitForTimeout(2500);
      await caption(page, "It also takes commands — /commands lists them");
      await type(page, "/commands");
      await page.waitForTimeout(3500);
    },
  }),
);

async function say(page: Page, text: string): Promise<void> {
  await focusGame(page);
  await page.keyboard.press("KeyC");
  await page.waitForTimeout(600);
  await type(page, text);
}

async function type(page: Page, text: string): Promise<void> {
  await clickUi(page, (element) => element.editable);
  await page.keyboard.type(text, { delay: 45 });
  await page.keyboard.press("Enter");
}
