import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { admin, give } from "../helpers/admin";
import { caption, chapter } from "../helpers/demo";
import {
  clickUi,
  findUi,
  finished,
  focusGame,
  holding,
  hoverUi,
  onQuest,
  probe,
  rightClickUi,
  waitFor,
  waitForWorld,
  type Snapshot,
  type UiElement,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, onStage, pick, readToChoices, talkTo, townsperson } from "../helpers/talk";

const LETTER = "icons/misc/envolop.png";
const SWORD = "icons/weapon_and_tool/iron_sword.png";
const BAIT = "icons/monster_part/monster_meat.png";

test(
  "Quests",
  chapter({
    summary: "Offers, a tracker and a log, deliveries, timers, and a reward that needs room",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await give(page, [["OrcTusk", 7]]);
    },
    play: async (page) => {
      await caption(page, "A ! over a townsperson means they have work for you");
      await waitFor(page, (snapshot) => townsperson(snapshot, "Tobb")?.marks.includes("QuestOffered"), "Tobb has no !");
      await talkTo(page, "Tobb");
      await readToChoices(page, 900);
      await caption(page, "Quests join the conversation as topics", { at: "top" });
      await page.waitForTimeout(1500);
      await pick(page, "A Letter for the Captain");
      await onStage(page, "LetterOffer");
      await caption(page, "A panel above the conversation lays out objectives and rewards. Accept or decline below", {
        at: "top",
      });
      await readToChoices(page, 1400);
      await page.waitForTimeout(2000);
      await pick(page, "I'll take it.");
      await waitFor(page, (snapshot) => onQuest(snapshot, "LetterForTheCaptain"), "the letter quest never started");
      await caption(page, "Accepted: a toast, the letter in your bag, and the quest on your tracker", { at: "top" });
      await hearOutTobb(page);
      await page.waitForTimeout(2000);

      await caption(page, "Quest items are marked: they can't be sold or dropped, and abandoning takes them back", {
        at: "top",
      });
      await focusGame(page);
      await page.keyboard.press("KeyI");
      await hoverUi(page, LETTER);
      await page.waitForTimeout(3500);
      await focusGame(page);
      await page.keyboard.press("KeyI");

      await caption(page, "Some quests run against the clock");
      await talkTo(page, "Tobb");
      await readToChoices(page, 600);
      await pick(page, "Low Tide");
      await onStage(page, "LowTideOffer");
      await readToChoices(page, 1200);
      await pick(page, "I'll hurry.");
      await waitFor(page, (snapshot) => onQuest(snapshot, "LowTide")?.left, "Low Tide never started its clock");
      await caption(page, "The tracker counts it down", { at: "top" });
      await page.waitForTimeout(3000);

      await caption(page, "L opens the quest log, grouped by category", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("KeyL");
      await waitFor(page, (snapshot) => findUi(snapshot, "Active quests"), "the quest log never opened");
      await page.waitForTimeout(1500);
      await clickUi(page, logRow(await probe(page), "Low Tide"));
      await page.waitForTimeout(2500);
      await caption(page, "Right-click a reward for its item card", { at: "top" });
      await rightClickUi(page, BAIT);
      await waitFor(page, ({ item_card }) => item_card?.item === "FishingBait", "the bait's card never opened");
      await page.waitForTimeout(3500);
      await focusGame(page);
      await page.keyboard.press("Escape");
      await waitFor(page, ({ item_card }) => item_card === null, "the item card never closed");
      await page.waitForTimeout(800);
      await clickUi(page, logRow(await probe(page), "A Letter for the Captain"));
      await page.waitForTimeout(2500);
      await clickUi(page, logRow(await probe(page), "Low Tide"));
      await page.waitForTimeout(1000);
      await caption(page, "Abandoning always asks, and Keep quest is the default — Enter keeps it", { at: "top" });
      await clickUi(page, "Abandon");
      await waitFor(page, (snapshot) => findUi(snapshot, "Abandon Low Tide?"), "abandoning never asked");
      await page.waitForTimeout(2500);
      await page.keyboard.press("Enter");
      await waitFor(page, (snapshot) => !findUi(snapshot, "Abandon Low Tide?"), "Enter never kept the quest");
      await page.waitForTimeout(1200);
      await clickUi(page, "Abandon");
      await waitFor(page, (snapshot) => findUi(snapshot, "Abandon quest"), "abandoning never asked again");
      await page.waitForTimeout(1200);
      await clickUi(page, "Abandon quest");
      await waitFor(page, (snapshot) => !onQuest(snapshot, "LowTide"), "Low Tide was never abandoned");
      await page.waitForTimeout(1500);
      await focusGame(page);
      await page.keyboard.press("Escape");
      await page.waitForTimeout(800);

      await caption(page, "The letter is for Bram: a ? marks where a quest is handed in");
      await waitFor(page, (snapshot) => townsperson(snapshot, "Bram")?.marks.includes("QuestReady"), "Bram has no ?");
      await talkTo(page, "Bram");
      await readToChoices(page, 600);
      await pick(page, "A Letter for the Captain");
      await onStage(page, "LetterThanks");
      await readToChoices(page, 1200);
      await caption(page, "The letter is what this choice costs", { at: "top" });
      await page.waitForTimeout(2000);
      await pick(page, "Tobb asked me to bring you this.");
      await waitFor(page, (snapshot) => finished(snapshot, "LetterForTheCaptain") === "Completed", "never delivered");
      await caption(page, "Delivered. The next quest in the chain shows what it still needs", { at: "top" });
      await page.waitForTimeout(1500);
      await talkTo(page, "Bram");
      await readToChoices(page, 600);
      await hoverUi(page, "Bats in the Belfry");
      await page.waitForTimeout(3000);
      await focusGame(page);
      await page.keyboard.press("Escape");
      await conversationOver(page);

      await caption(page, "Mara pays for orc tusks — and their chief");
      await talkTo(page, "Mara");
      await readToChoices(page, 600);
      await pick(page, "Tusks for the Chief");
      await onStage(page, "TusksOffer");
      await caption(page, "Rewards can include a pick, chosen when you return", { at: "top" });
      await readToChoices(page, 1400);
      await page.waitForTimeout(1500);
      await pick(page, "Consider it done.");
      await waitFor(page, (snapshot) => onQuest(snapshot, "TusksForTheChief"), "Mara's quest never started");
      await page.waitForTimeout(1500);

      await caption(page, "(Admin shortcut: the chief falls, and the bag fills up)", { at: "top" });
      const room = 25 - (await probe(page)).bag.length;
      await admin(page, [
        ["/quest TusksForTheChief,ready", /TusksForTheChief: Ready/],
        [`/give RustySword,${room}`, /gave \d+ Rusty Sword/],
      ]);
      await waitFor(page, (snapshot) => onQuest(snapshot, "TusksForTheChief")?.ready, "the quest never got ready");

      await talkTo(page, "Mara");
      await readToChoices(page, 600);
      await pick(page, "Tusks for the Chief");
      await onStage(page, "TusksThanks");
      await readToChoices(page, 1000);
      await caption(page, "A full bag locks every pick — space is counted after the tusks are handed in", {
        at: "top",
      });
      await hoverUi(page, "I'll take the Bone Shield.");
      await page.waitForTimeout(3500);
      await pick(page, "Not yet.");
      await conversationOver(page);

      await caption(page, "Make some room: Ctrl-click drops", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("KeyI");
      await page.waitForTimeout(800);
      for (let dropped = 0; dropped < 2; dropped++) {
        const before = holding(await probe(page), "RustySword");
        await page.keyboard.down("Control");
        await clickUi(page, SWORD);
        await page.keyboard.up("Control");
        await waitFor(page, (snapshot) => holding(snapshot, "RustySword") < before, "the sword never dropped");
      }
      await focusGame(page);
      await page.keyboard.press("KeyI");

      await talkTo(page, "Mara");
      await readToChoices(page, 600);
      await pick(page, "Tusks for the Chief");
      await onStage(page, "TusksThanks");
      await readToChoices(page, 800);
      await caption(page, "Now there's room for the pick", { at: "top" });
      await pick(page, "I'll take the Bone Shield.");
      await waitFor(page, (snapshot) => finished(snapshot, "TusksForTheChief") === "Completed", "never handed in");
      await page.waitForTimeout(2500);

      await caption(page, "Finished quests move to Completed in the log", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("KeyL");
      await waitFor(page, (snapshot) => findUi(snapshot, "Completed"), "the quest log never opened");
      await clickUi(page, "Completed");
      await page.waitForTimeout(3000);
      await focusGame(page);
      await page.keyboard.press("Escape");
    },
  }),
);

// The same title shows on the tracker and in the log's detail pane; a row sits under the list's header.
function logRow(snapshot: Snapshot, title: string): (element: UiElement) => boolean {
  const header = findUi(snapshot, "Active quests");
  if (!header) throw new Error("the quest log is closed");
  return (element) => element.text === title && element.y > header.y && Math.abs(element.x - header.x) < 64;
}

async function hearOutTobb(page: Page): Promise<void> {
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    const { stage } = await probe(page);
    if (stage?.node === "TobbNews") {
      await readToChoices(page, 1200);
      await focusGame(page);
      await page.keyboard.press("Digit1");
      await conversationOver(page);
      return;
    }
    await page.waitForTimeout(100);
  }
}
