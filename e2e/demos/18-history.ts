import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { give } from "../helpers/admin";
import { caption, chapter } from "../helpers/demo";
import {
  arrivedIn,
  clickUi,
  focusGame,
  historyTab,
  hoverUi,
  onQuest,
  probe,
  travelTo,
  waitFor,
  waitForWorld,
  walkInto,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, onStage, pick, readToChoices, talkTo } from "../helpers/talk";

const HISTORY = "icons/misc/book_3.png";

test(
  "History",
  chapter({
    summary: "Everything said, shown and gained, kept in one place",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await give(page, [["RoadPass", 1]]);
    },
    play: async (page) => {
      await caption(page, "Everything you say, see and get is kept: a talk with Tobb, the gulls out on the pier…");
      await talkTo(page, "Tobb");
      await readToChoices(page, 900);
      await pick(page, "A Letter for the Captain");
      await onStage(page, "LetterOffer");
      await readToChoices(page, 1000);
      await pick(page, "I'll take it.");
      await waitFor(page, (snapshot) => onQuest(snapshot, "LetterForTheCaptain"), "the letter quest never started");
      await leaveTobb(page);

      await walkInto(page, "ferry-pier");
      await waitFor(page, ({ history }) => history.records.some(({ by }) => by === "Gulls"), "the gulls never squabbled");

      await caption(page, "History has its own slot in the toolbar, or press H");
      await hoverUi(page, HISTORY);
      await page.waitForTimeout(1500);
      await clickUi(page, HISTORY);
      await waitFor(page, ({ history }) => history.open, "history never opened");
      await caption(page, "All of it, in order: conversations grouped, what you heard, what you gained", {
        at: "top",
      });
      await page.waitForTimeout(4000);
      for (const [tab, says] of [
        ["Talk", "Tabs filter by what a record is about: what was said…"],
        ["Quests", "…quests…"],
        ["Items", "…what came and went…"],
        ["Notifications", "…and every notification, whether or not you were looking"],
      ]) {
        await caption(page, says, { at: "top" });
        await openTab(page, tab);
        await page.waitForTimeout(2500);
      }
      await openTab(page, "All");
      await focusGame(page);
      await page.keyboard.press("Escape");
      await waitFor(page, ({ history }) => !history.open, "history never closed");

      await caption(page, "History stays with your character, across the strait too");
      const road = (await probe(page)).portals.find((portal) => portal.name === "forest-road")!;
      await travelTo(page, road.at);
      await waitFor(page, (snapshot) => arrivedIn(snapshot, "Forest"), "never reached the forest");
      await waitFor(page, ({ notifications }) => notifications.intro?.title === "The forest", "the forest never spoke", 20_000);
      await page.waitForTimeout(2000);
      await focusGame(page);
      await page.keyboard.press("KeyH");
      await waitFor(
        page,
        ({ history }) => history.open && history.records.some((record) => record.mark === "Arrived"),
        "history never marked the crossing",
      );
      await caption(page, "Changing area adds a divider, so you can tell where things happened", { at: "top" });
      await page.waitForTimeout(4000);
      await page.keyboard.press("KeyH");
      await waitFor(page, ({ history }) => !history.open, "history never closed");
    },
  }),
);

async function openTab(page: Page, title: string): Promise<void> {
  const tab = await historyTab(page, title);
  await clickUi(page, (element) => element.text === tab.text && element.x === tab.x && element.y === tab.y);
  await waitFor(page, ({ history }) => history.tab === title, `the ${title} tab never opened`);
}

async function leaveTobb(page: Page): Promise<void> {
  for (let tries = 0; tries < 3; tries++) {
    const { stage } = await probe(page);
    if (!stage) return;
    await readToChoices(page, 900);
    await focusGame(page);
    await page.keyboard.press("Digit1");
    await page.waitForTimeout(1500);
  }
  await conversationOver(page);
}
