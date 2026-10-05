import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { caption, chapter } from "../helpers/demo";
import { clickUi, findUi, focusGame, hoverTile, hoverUi, probe, waitFor, waitForWorld } from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, interactWith, onStage, pick, readToChoices, talkTo } from "../helpers/talk";

test(
  "Conversation",
  chapter({
    summary: "Talk to the townsfolk: busts, choices, costs, history",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await giveGold(page, 12);
    },
    play: async (page) => {
      await caption(page, "Click a townsperson to talk — you walk into reach and the conversation opens");
      await talkTo(page, "Tobb");
      await caption(page, "Busts show who is speaking. Space, Enter or a click reads on");
      await readToChoices(page, 1400);
      await waitFor(page, ({ stage }) => stage?.waiting === "TobbNews", "Tobb never queued his news");
      await caption(page, "Tobb has news for you — it waits its turn, shown on the frame", { at: "top" });
      await page.waitForTimeout(2500);
      await caption(page, "↑ ↓ choose and Enter picks", { at: "top" });
      await page.keyboard.press("ArrowDown");
      await page.waitForTimeout(700);
      await page.keyboard.press("ArrowUp");
      await page.waitForTimeout(700);
      await page.keyboard.press("Enter");
      await onStage(page, "TobbCatch");
      await readToChoices(page, 1200);
      await caption(page, "…or click a choice", { at: "top" });
      await pick(page, "Good luck out there.");
      await onStage(page, "TobbNews");
      await caption(page, "When one conversation ends, the one waiting begins", { at: "top" });
      await readToChoices(page, 1400);
      await page.keyboard.press("Digit1");
      await conversationOver(page);

      await caption(page, "While you talk, clicks on the world are ignored", { at: "top" });
      await talkTo(page, "Grisha");
      const me = (await probe(page)).me!;
      await hoverTile(page, [me.at[0] + 2, me.at[1] - 4]);
      await page.mouse.down();
      await page.mouse.up();
      await page.waitForTimeout(1500);
      await readToChoices(page, 1200);
      await caption(page, "Choices show what they cost, and what you have", { at: "top" });
      await hoverUi(page, "Buy a round for the room.");
      await page.waitForTimeout(2000);
      await pick(page, "Buy a round for the room.");
      await onStage(page, "GrishaRound");
      await readToChoices(page, 1400);
      await caption(page, "Can't afford it? The choice shakes and says why", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("Digit1");
      await page.waitForTimeout(2500);
      await caption(page, "H opens the history of everything said", { at: "top" });
      await page.keyboard.press("KeyH");
      await waitFor(page, ({ history }) => history, "history never opened");
      await page.waitForTimeout(3000);
      await caption(page, "Esc closes the history, then leaves the conversation", { at: "top" });
      await page.keyboard.press("Escape");
      await page.waitForTimeout(1000);
      await page.keyboard.press("Escape");
      await conversationOver(page);
      await page.waitForTimeout(1200);

      await caption(page, "Things on the map can start conversations too — a chest washed up by the tide");
      await interactWith(page, "TideChest");
      await readToChoices(page, 1400);
      await pick(page, "Close the lid.");
      await conversationOver(page);
      await page.waitForTimeout(1500);
    },
  }),
);

async function giveGold(page: Page, count: number): Promise<void> {
  await focusGame(page);
  await page.keyboard.press("KeyC");
  await clickUi(page, "Admin");
  await page.waitForTimeout(1200);
  await clickUi(page, (element) => element.editable);
  await page.waitForTimeout(400);
  await page.keyboard.type(`/give Gold,${count}`, { delay: 45 });
  await page.keyboard.press("Enter");
  await waitFor(page, (snapshot) => findUi(snapshot, new RegExp(`gave ${count} Gold$`)), "the gold never arrived");
  await focusGame(page);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);
  await page.keyboard.press("Escape");
  await waitFor(page, (snapshot) => !findUi(snapshot, "Admin"), "the terminal never closed");
}
