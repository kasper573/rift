import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { give } from "../helpers/admin";
import { caption, chapter, soundscapeMixer } from "../helpers/demo";
import {
  clickUi,
  doubleClickUi,
  dragUi,
  findUi,
  focusGame,
  holding,
  hoverUi,
  leaveConversation,
  outsideCard,
  probe,
  rightClickUi,
  waitFor,
  waitForWorld,
  walkTo,
  type Snapshot,
  type UiElement,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { lineRead, talkTo } from "../helpers/talk";

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
        ["rusty_sword", 1],
        ["bone_shield", 1],
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
      await waitFor(page, ({ item_card }) => item_card?.item === "bone_shield", "the shield's card never opened");
      await page.waitForTimeout(3500);
      await caption(page, "One card at a time: right-click another item and the card follows");
      await rightClickUi(page, outsideCard(await probe(page), SWORD));
      await waitFor(page, ({ item_card }) => item_card?.item === "rusty_sword", "the sword's card never opened");
      await page.waitForTimeout(2500);
      await caption(page, "Double-click an item to use it — or, for gear, to wear it");
      await doubleClickUi(page, outsideCard(await probe(page), SWORD));
      await waitFor(page, (snapshot) => holding(snapshot, "rusty_sword") === 0, "the sword was never worn");
      await page.waitForTimeout(2000);
      await caption(page, "Worn gear has a card too…");
      await rightClickUi(page, outsideCard(await probe(page), SHIELD));
      await waitFor(page, ({ item_card }) => item_card?.item === "bone_shield", "the shield's card never opened");
      await page.waitForTimeout(800);
      await rightClickUi(page, outsideCard(await probe(page), SWORD));
      await waitFor(page, ({ item_card }) => item_card?.item === "rusty_sword", "the worn sword's card never opened");
      await page.waitForTimeout(2000);
      await caption(page, "…and a double-click takes it off");
      await doubleClickUi(page, outsideCard(await probe(page), SWORD));
      await waitFor(page, (snapshot) => holding(snapshot, "rusty_sword") === 1, "the sword was never taken off");
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
      await caption(page, "Sound has a volume each: master, music, ambience, effects and voice, all halfway to start");
      await hoverUi(page, /^reduced motion/);
      await page.mouse.wheel(0, 400);
      await page.waitForTimeout(1500);
      await caption(page, "Halfway is the mix as balanced, with room to turn any of them up or down");
      await page.waitForTimeout(2500);
      const hideMixer = await soundscapeMixer(page);
      await caption(page, "Music is the soundscape's first channel: turned down, the harbour theme fades out");
      await setVolume(page, "Music", 0);
      await page.waitForTimeout(3500);
      await caption(page, "Ambience is every other channel: down goes the surf, and the island falls silent");
      await setVolume(page, "Ambience", 0);
      await page.waitForTimeout(3500);
      await caption(page, "Effects are the game's sounds: in the quiet, your footsteps…");
      const home = (await probe(page)).me!.at;
      await walkTo(page, [home[0] + 3, home[1]]);
      await page.waitForTimeout(1000);
      await caption(page, "…which go quiet too with effects turned down");
      await setVolume(page, "Effects", 0);
      await walkTo(page, home);
      await page.waitForTimeout(1000);
      await caption(page, "Back to halfway, everything plays as balanced again");
      for (const fader of ["Music", "Ambience", "Effects"]) await setVolume(page, fader, 50);
      await page.waitForTimeout(3000);
      await hideMixer();
      await caption(page, "Where you put them is remembered for next time");
      await page.waitForTimeout(2500);
      await caption(page, "Esc closes the window you used last");
      await focusGame(page);
      for (const _ of LAYOUT) {
        await page.keyboard.press("Escape");
        await page.waitForTimeout(600);
      }
      await caption(page, "Voice is how characters speak: at halfway, Grisha's greeting babbles…");
      await talkTo(page, "grisha");
      await lineRead(page);
      await page.waitForTimeout(1500);
      await leaveConversation(page);
      await page.keyboard.press("KeyO");
      await hoverUi(page, /^reduced motion/);
      await page.mouse.wheel(0, 400);
      await page.waitForTimeout(800);
      await setVolume(page, "Voice", 0);
      await focusGame(page);
      await page.keyboard.press("Escape");
      await caption(page, "…and with voice turned down, the same greeting is silent");
      await talkTo(page, "grisha");
      await lineRead(page);
      await page.waitForTimeout(1500);
      await leaveConversation(page);
    },
  }),
);

const VOLUME_TOLERANCE = 0.02;
const PAST_EMPTY = 300;
let volumeTrackWidth = 0;

// A thumb sits over its value along the track, so the track's width turns a value into a drag distance.
async function setVolume(page: Page, fader: string, percent: number): Promise<void> {
  const thumbIn = (snapshot: Snapshot) => {
    const name = findUi(snapshot, fader)!;
    const middle = name.y + name.height / 2;
    return (element: UiElement) => element.slider !== null && Math.abs(element.y + element.height / 2 - middle) < 12;
  };
  const target = percent / 100;
  const before = await probe(page);
  const thumb = before.ui.find(thumbIn(before))!;
  const by = target === 0 ? -PAST_EMPTY : (target - thumb.slider!) * volumeTrackWidth;
  await dragUi(page, thumbIn(before), { x: by, y: 0 });
  const moved = await waitFor(
    page,
    (snapshot) => {
      const now = snapshot.ui.find(thumbIn(snapshot));
      return now && Math.abs(now.slider! - target) <= VOLUME_TOLERANCE && now;
    },
    `the ${fader} volume never reached ${percent}%`,
  );
  if (target === 0 && thumb.slider! > 0) {
    volumeTrackWidth = (thumb.x - moved.x) / thumb.slider!;
  }
}

