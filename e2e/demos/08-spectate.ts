import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { admin } from "../helpers/admin";
import { caption, chapter, enterWorld } from "../helpers/demo";
import {
  arrivedIn,
  clickUi,
  closestTile,
  findUi,
  focusGame,
  onQuest,
  probe,
  travelTo,
  waitFor,
  waitForWorld,
  walkTo,
  type Tile,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { onStage, pick, readToChoices, talkTo } from "../helpers/talk";

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
      await signIn(explorer, await provisionAccount(explorer, ["admin"]));
      await clickUi(explorer, "Play");
      await waitForWorld(explorer, loadReference("island.png"));
      await admin(explorer, [["/give road_pass,1", /gave 1 Road Pass/]]);
      const road = (await probe(explorer)).portals.find((portal) => portal.name === "forest-road")!;
      await travelTo(explorer, road.at);
      await waitFor(explorer, (snapshot) => arrivedIn(snapshot, road.to), "the explorer never reached the forest");
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
      await caption(page, "That includes their conversations, read along without a way to answer");
      await talkTo(islander, "tobb");
      await onStage(page, "tobb_hello");
      await readToChoices(islander, 1800);
      await pick(islander, "A Letter for the Captain");
      await onStage(page, "letter_offer");
      await caption(page, "The quest panel and the choices show, but only the player picks", { at: "top" });
      await readToChoices(islander, 1800);
      await page.waitForTimeout(2500);
      await pick(islander, "I'll take it.");
      await waitFor(page, (snapshot) => onQuest(snapshot, "letter_for_the_captain"), "the quest never showed");
      await caption(page, "Their quest tracker follows along, and so does their feed in the corner", { at: "top" });
      await waitFor(
        page,
        ({ feed }) => feed.includes("Quest accepted · A Letter for the Captain"),
        "the spectator's feed never showed the quest",
      );
      await page.waitForTimeout(3000);
      await caption(page, "H opens their history, and says whose it is", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("KeyH");
      await waitFor(
        page,
        ({ history }) => history.open && history.records.some((record) => record.text.includes("A Letter for the Captain")),
        "the watched player's history never showed",
      );
      await page.waitForTimeout(3500);
      await page.keyboard.press("KeyH");
      await waitFor(page, ({ history }) => !history.open, "history never closed");
      await caption(page, "So do their shops, with every button off");
      await talkTo(islander, "mara");
      await readToChoices(islander, 900);
      await pick(islander, "Show me your wares.");
      await waitFor(page, ({ shop }) => shop?.shop === "mara_wares", "the shop never showed");
      await page.waitForTimeout(4000);
      await focusGame(islander);
      await islander.keyboard.press("Escape");
      await waitFor(page, ({ shop }) => !shop, "the spectator kept the shop open");
      await islander.keyboard.press("Escape");
      await waitFor(page, ({ stage }) => !stage, "the spectator kept the conversation open");
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
