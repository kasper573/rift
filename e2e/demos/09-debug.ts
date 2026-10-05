import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { focusGame } from "../helpers/game";

test(
  "Debug views",
  chapter({
    summary: "Overlays that show what the game sees",
    play: async (page) => {
      await focusGame(page);
      await caption(page, "F1 shows the walkable navigation graph");
      await page.keyboard.press("F1");
      await page.waitForTimeout(4000);
      await caption(page, "Again for the objects that can hide an actor");
      await page.keyboard.press("F1");
      await page.waitForTimeout(3000);
      await caption(page, "Again for the safe zones, where monsters never go");
      await page.keyboard.press("F1");
      await page.waitForTimeout(4000);
      await page.keyboard.press("F1");
      await caption(page, "F2 shows every actor's hitbox");
      await page.keyboard.press("F2");
      await page.waitForTimeout(4000);
      await page.keyboard.press("F2");
    },
  }),
);
