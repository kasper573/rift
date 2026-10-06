import { test } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { give } from "../helpers/admin";
import { caption, chapter } from "../helpers/demo";
import {
  clickUi,
  doubleClickUi,
  dragUi,
  findUi,
  focusGame,
  holding,
  hoverUi,
  outsideCard,
  probe,
  rightClickUi,
  waitFor,
  waitForWorld,
} from "../helpers/game";
import { loadReference } from "../helpers/image";

// Each window opens at the same spot; these spread them over the corners of a 1280×720 view.
const LAYOUT = [
  { key: "KeyI", title: "Inventory", by: { x: -330, y: -230 } },
  { key: "KeyE", title: "Equipment", by: { x: 440, y: -230 } },
  { key: "KeyK", title: "Stats", by: { x: -330, y: 40 } },
  { key: "KeyO", title: "Settings", by: { x: 440, y: 40 } },
];

const SWORD = "icons/weapon_and_tool/iron_sword.png";
const SHIELD = "icons/weapon_and_tool/wooden_shield.png";

test(
  "Windows",
  chapter({
    summary: "Inventory, equipment, stats and settings, laid out your way",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await give(page, [
        ["RustySword", 1],
        ["BoneShield", 1],
      ]);
    },
    play: async (page) => {
      await caption(page, "Every window has a hotkey: I, E, K, L, O and C");
      await focusGame(page);
      for (const { key, title, by } of LAYOUT) {
        await page.keyboard.press(key);
        await page.waitForTimeout(900);
        if (title === "Inventory") await caption(page, "Drag a window by its title to move it");
        await dragUi(page, title, by);
        await page.waitForTimeout(700);
      }
      await caption(page, "Right-click an item for its card: what it does, and a line of lore");
      await rightClickUi(page, outsideCard(await probe(page), SHIELD));
      await waitFor(page, ({ item_card }) => item_card?.item === "BoneShield", "the shield's card never opened");
      await page.waitForTimeout(3500);
      await caption(page, "One card at a time: right-click another item and the card follows");
      await rightClickUi(page, outsideCard(await probe(page), SWORD));
      await waitFor(page, ({ item_card }) => item_card?.item === "RustySword", "the sword's card never opened");
      await page.waitForTimeout(2500);
      await caption(page, "Double-click an item to use it — or, for gear, to wear it");
      await doubleClickUi(page, outsideCard(await probe(page), SWORD));
      await waitFor(page, (snapshot) => holding(snapshot, "RustySword") === 0, "the sword was never worn");
      await page.waitForTimeout(2000);
      await caption(page, "Worn gear has a card too…");
      await rightClickUi(page, outsideCard(await probe(page), SHIELD));
      await waitFor(page, ({ item_card }) => item_card?.item === "BoneShield", "the shield's card never opened");
      await page.waitForTimeout(800);
      await rightClickUi(page, outsideCard(await probe(page), SWORD));
      await waitFor(page, ({ item_card }) => item_card?.item === "RustySword", "the worn sword's card never opened");
      await page.waitForTimeout(2000);
      await caption(page, "…and a double-click takes it off");
      await doubleClickUi(page, outsideCard(await probe(page), SWORD));
      await waitFor(page, (snapshot) => holding(snapshot, "RustySword") === 1, "the sword was never taken off");
      await page.waitForTimeout(2000);
      await caption(page, "Esc closes the card before any window");
      await focusGame(page);
      await page.keyboard.press("Escape");
      await waitFor(page, ({ item_card }) => item_card === null, "the item card never closed");
      await page.waitForTimeout(1200);
      await caption(page, "Windows snap to a grid — toggle it in Settings");
      await clickUi(page, /^ui snapping/);
      await page.waitForTimeout(1500);
      await clickUi(page, /^ui snapping/);
      await page.waitForTimeout(1200);
      await caption(page, "Text speed is a slider, in letters a second");
      const before = findUi(await probe(page), /^text speed/)!.text;
      await dragUi(page, (element) => element.slider !== null, { x: 70, y: 0 });
      await waitFor(page, (snapshot) => findUi(snapshot, /^text speed/)?.text !== before, "the text speed never changed");
      await page.waitForTimeout(1200);
      await caption(page, "Reduced motion is a setting too");
      await clickUi(page, /^reduced motion/);
      await page.waitForTimeout(1200);
      await caption(page, "Sound has a volume each for everything, music, voices and effects");
      await hoverUi(page, /^reduced motion/);
      await page.mouse.wheel(0, 400);
      await page.waitForTimeout(1200);
      const voice = findUi(await probe(page), /^voice volume/)!;
      await dragUi(page, (element) => element.slider !== null && element.y > voice.y && element.y < voice.y + 40, {
        x: -80,
        y: 0,
      });
      await waitFor(page, (snapshot) => findUi(snapshot, /^voice volume/)?.text !== voice.text, "the voice volume never changed");
      await page.waitForTimeout(1500);
      await caption(page, "Where you put them is remembered for next time");
      await page.waitForTimeout(2500);
      await caption(page, "Esc closes the window you used last");
      await focusGame(page);
      for (const _ of LAYOUT) {
        await page.keyboard.press("Escape");
        await page.waitForTimeout(600);
      }
    },
  }),
);
