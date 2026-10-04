import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { clickUi, dragUi, focusGame } from "../helpers/game";

// Each window opens at the same spot; these spread them over the corners of a 1280×720 view.
const LAYOUT = [
  { key: "KeyI", title: "Inventory", by: { x: -330, y: -230 } },
  { key: "KeyE", title: "Equipment", by: { x: 440, y: -230 } },
  { key: "KeyK", title: "Stats", by: { x: -330, y: 40 } },
  { key: "KeyO", title: "Settings", by: { x: 440, y: 40 } },
];

test(
  "Windows",
  chapter({
    summary: "Inventory, equipment, stats and settings, laid out your way",
    play: async (page) => {
      await caption(page, "Every window has a hotkey: I, E, K, O and C");
      await focusGame(page);
      for (const { key, title, by } of LAYOUT) {
        await page.keyboard.press(key);
        await page.waitForTimeout(900);
        if (title === "Inventory") await caption(page, "Drag a window by its title to move it");
        await dragUi(page, title, by);
        await page.waitForTimeout(700);
      }
      await caption(page, "Windows snap to a grid — toggle it in Settings");
      await clickUi(page, /^ui snapping/);
      await page.waitForTimeout(1500);
      await clickUi(page, /^ui snapping/);
      await page.waitForTimeout(1200);
      await caption(page, "Where you put them is remembered for next time");
      await page.waitForTimeout(2500);
      await focusGame(page);
      for (const { key } of LAYOUT) {
        await page.keyboard.press(key);
        await page.waitForTimeout(400);
      }
    },
  }),
);
