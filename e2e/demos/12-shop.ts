import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { give } from "../helpers/admin";
import { caption, chapter } from "../helpers/demo";
import {
  clickUi,
  dragUi,
  dragUiOnto,
  findUi,
  focusGame,
  holding,
  probe,
  rightClickUi,
  waitFor,
  waitForWorld,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, pick, readToChoices, shopAt, talkTo } from "../helpers/talk";

test(
  "Shops",
  chapter({
    summary: "Buy, sell and buy back — from the townsfolk, and from a box on the pier",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await give(page, [
        ["Gold", 40],
        ["OrcTusk", 2],
        ["Bone", 4],
        ["RustySword", 1],
      ]);
    },
    play: async (page) => {
      await caption(page, "Mara keeps a shop: ask, and her wares open above the conversation");
      await talkTo(page, "Mara");
      await readToChoices(page, 900);
      await pick(page, "Show me your wares.");
      await waitFor(page, ({ shop }) => shop?.shop === "MaraWares", "Mara's shop never opened");
      await page.waitForTimeout(1500);
      await caption(page, "It belongs to the conversation: fixed above it, with no close button", { at: "top" });
      const before = findUi(await probe(page), "Mara's Wares");
      await dragUi(page, "Mara's Wares", { x: -300, y: 0 });
      await page.waitForTimeout(1500);
      const after = findUi(await probe(page), "Mara's Wares");
      if (!before || !after || before.x !== after.x || before.y !== after.y) throw new Error("the shop panel moved");

      await caption(page, "Prices are lists of items — each part turns red when you can't cover it", { at: "top" });
      await clickUi(page, "Greater Health Potion");
      await page.waitForTimeout(2500);
      await caption(page, "The list stays as narrow as its wares, and browsing never shifts it", { at: "top" });
      const listed = await rowX(page, "Bone Shield");
      for (const ware of ["Bone Shield", "Health Potion", "Greater Health Potion"]) {
        await clickUi(page, (element) => element.text === ware && element.x <= listed + 1);
        await page.waitForTimeout(900);
        if ((await rowX(page, "Bone Shield")) !== listed) throw new Error("the ware list shifted");
      }
      await caption(page, "Mara's shop opts into reactions — try buying anyway", { at: "top" });
      await clickUi(page, "Can't afford");
      await waitFor(page, (snapshot) => findUi(snapshot, /purse is heavier/), "Mara never reacted");
      await page.waitForTimeout(3000);

      await caption(page, "Buying trades the price for the goods", { at: "top" });
      await clickUi(page, "Health Potion");
      await page.waitForTimeout(800);
      await clickUi(page, "Buy");
      await waitFor(page, (snapshot) => holding(snapshot, "HealthPotion") === 1, "the potion never arrived");
      await page.waitForTimeout(2000);

      await caption(page, "Limited stock is yours alone, and restocks on a timer", { at: "top" });
      await clickUi(page, "Bone Shield");
      await page.waitForTimeout(1200);
      await clickUi(page, "Buy");
      await waitFor(
        page,
        ({ shop }) => shop?.offers.find((offer) => offer.item === "BoneShield")?.left === 0,
        "the shield never sold out",
      );
      await page.waitForTimeout(2500);


      await caption(page, "With a shop open, your bag shows what sells and dims what doesn't", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("KeyI");
      await dragUi(page, "Inventory", { x: 384, y: -260 });
      await page.waitForTimeout(2500);
      await caption(page, "Right-click to sell…", { at: "top" });
      await rightClickUi(page, "icons/monster_part/skull.png");
      await waitFor(page, (snapshot) => holding(snapshot, "OrcTusk") === 0, "the tusks never sold");
      await page.waitForTimeout(1500);
      await caption(page, "…or drag onto the shop. Equipment asks first", { at: "top" });
      await dragUiOnto(page, "icons/weapon_and_tool/iron_sword.png", /^Buys /);
      await waitFor(page, (snapshot) => findUi(snapshot, "Sell Rusty Sword?"), "selling the sword never asked");
      await page.waitForTimeout(1500);
      await clickUi(page, "Sell");
      await waitFor(page, (snapshot) => holding(snapshot, "RustySword") === 0, "the sword never sold");
      await page.waitForTimeout(1500);

      await caption(page, "Changed your mind? Buy it back for exactly what she paid", { at: "top" });
      await clickUi(page, /^Buy back \(/);
      await page.waitForTimeout(1200);
      await clickUi(page, "Rusty Sword");
      await page.waitForTimeout(800);
      await clickUi(page, "Buy back");
      await waitFor(page, (snapshot) => holding(snapshot, "RustySword") === 1, "the sword never came back");
      await page.waitForTimeout(1500);

      await caption(page, "Leaving the conversation closes the shop with it", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("Escape");
      await conversationOver(page);
      await waitFor(page, ({ shop }) => shop === null, "the shop outlived the conversation");
      await page.waitForTimeout(1000);

      await caption(page, "Wren collects bones and wings, and pays in her own Bone Tokens");
      await talkTo(page, "Wren");
      await readToChoices(page, 900);
      await pick(page, "I've brought bones.");
      await waitFor(page, ({ shop }) => shop?.shop === "BoneExchange", "Wren's exchange never opened");
      await caption(page, "Her shop has no reactions: trades pass in silence, and your bag lights up what she takes", {
        at: "top",
      });
      await page.waitForTimeout(3000);
      await rightClickUi(page, "icons/monster_part/bone.png");
      await waitFor(page, (snapshot) => holding(snapshot, "BoneToken") === 4, "the bones never sold");
      await page.waitForTimeout(2000);
      await focusGame(page);
      await page.keyboard.press("Escape");
      await waitFor(page, ({ shop }) => shop === null, "leaving never closed the exchange");
      await page.keyboard.press("KeyI");

      await caption(page, "Even a box on the pier can keep a shop, inside a conversation of its own");
      await shopAt(page, "HonestyBox");
      await page.waitForTimeout(1500);
      await clickUi(page, "Fish Steak");
      await page.waitForTimeout(800);
      await clickUi(page, "Buy");
      await waitFor(page, (snapshot) => holding(snapshot, "FishSteak") === 1, "the fish steak never arrived");
      await page.waitForTimeout(2500);
    },
  }),
);

async function rowX(page: Page, ware: string): Promise<number> {
  return Math.min(...(await probe(page)).ui.filter((element) => element.text === ware).map((element) => element.x));
}
