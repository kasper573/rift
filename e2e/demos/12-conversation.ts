import { test } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { give } from "../helpers/admin";
import { caption, chapter } from "../helpers/demo";
import { clickUi, focusGame, hoverTile, hoverUi, probe, waitFor, waitForWorld } from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, interactWith, onStage, pick, readToChoices, talkTo } from "../helpers/talk";

test(
  "Conversation",
  chapter({
    summary: "Talk to the townsfolk: busts, choices, prices, refusals, history",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await give(page, [["gold", 12]]);
    },
    play: async (page) => {
      await caption(page, "Click a townsperson to talk — you walk into reach and the conversation opens");
      await talkTo(page, "tobb");
      await caption(page, "Busts show who is speaking. Space, Enter or a click reads on");
      await readToChoices(page, 1400);
      await caption(page, "No buttons on the frame: Esc leaves, and the choices say the rest", { at: "top" });
      await page.waitForTimeout(2500);
      await caption(page, "A choice shows its words and nothing more, unless the writer reveals more", { at: "top" });
      await page.waitForTimeout(2000);
      await caption(page, "↑ ↓ choose and Enter picks", { at: "top" });
      const { stage } = await probe(page);
      const catchIndex = stage!.choices.findIndex((choice) => choice.label === "Caught anything good today?");
      for (let step = 0; step <= catchIndex; step++) {
        await page.keyboard.press("ArrowDown");
        await page.waitForTimeout(500);
      }
      await page.keyboard.press("ArrowUp");
      await page.waitForTimeout(700);
      await page.keyboard.press("Enter");
      await onStage(page, "tobb_catch");
      await readToChoices(page, 1200);
      await caption(page, "…or click a choice", { at: "top" });
      await pick(page, "Good luck out there.");
      await onStage(page, "tobb_news");
      await caption(page, "Tobb held his news while you talked — once you're free, he calls you over", { at: "top" });
      await readToChoices(page, 1400);
      await page.keyboard.press("Digit1");
      await conversationOver(page);

      await caption(page, "While you talk, the map ignores the mouse: a plain pointer, and clicks do nothing", {
        at: "top",
      });
      await talkTo(page, "grisha");
      const me = (await probe(page)).me!;
      const offStage: [number, number] = [me.at[0] + 2, me.at[1] - 4];
      await hoverTile(page, offStage);
      await page.mouse.down();
      await page.mouse.up();
      await page.waitForTimeout(1500);
      await readToChoices(page, 1200);
      await caption(page, "Over the conversation the cursor is its own: an arrow on the words, a hand on the choices", {
        at: "top",
      });
      await hoverUi(page, /Driftwood Inn/);
      await page.waitForTimeout(1800);
      await hoverUi(page, "Chat.");
      await page.waitForTimeout(1800);
      await hoverTile(page, offStage);
      await page.waitForTimeout(1200);
      await caption(page, "Choices branch into a tree: Chat opens a menu of its own", { at: "top" });
      await pick(page, "Chat.");
      await onStage(page, "grisha_chat");
      await readToChoices(page, 1200);
      await pick(page, "Tell me about this island.");
      await onStage(page, "grisha_island");
      await readToChoices(page, 1200);
      await caption(page, "Ok leads back up to the chat menu", { at: "top" });
      await pick(page, "Ok.");
      await onStage(page, "grisha_chat");
      await readToChoices(page, 1000);
      await pick(page, "Any rumours?");
      await onStage(page, "grisha_rumour_wren");
      await readToChoices(page, 1200);
      await caption(page, "…and a branch can go deeper still", { at: "top" });
      await pick(page, "What does she do with them?");
      await onStage(page, "grisha_wren_secret");
      await readToChoices(page, 1200);
      await pick(page, "Ok.");
      await onStage(page, "grisha_chat");
      await readToChoices(page, 1000);
      await caption(page, "Never mind goes back to the top", { at: "top" });
      await pick(page, "Never mind.");
      await onStage(page, "grisha_anything_else");
      await readToChoices(page, 1000);
      await caption(page, "An inn has its prices on the wall: Grisha's menu shows what each choice costs", { at: "top" });
      await hoverUi(page, "Buy a round for the room.");
      await page.waitForTimeout(2000);
      await pick(page, "Buy a round for the room.");
      await onStage(page, "grisha_round");
      await waitFor(page, ({ feed }) => feed.includes("−10 Gold"), "the round's price never showed in the feed");
      await caption(page, "What it took shows in the corner, over Grisha's bust", { at: "top" });
      await readToChoices(page, 1400);
      await page.waitForTimeout(1500);
      await caption(page, "Can't afford it? Pick it anyway: it shakes, an error says why, and you can pick again", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("Digit1");
      await waitFor(page, ({ notifications }) => notifications.error?.startsWith("Needs") ?? false, "the refused pick never said why");
      await page.waitForTimeout(2500);
      await caption(page, "History is a window like any other: H opens it here too, over the conversation", { at: "top" });
      await page.keyboard.press("KeyH");
      await waitFor(page, ({ history }) => history.open, "history never opened");
      await page.waitForTimeout(3500);
      await caption(page, "Esc closes the history, then leaves the conversation", { at: "top" });
      await page.keyboard.press("Escape");
      await page.waitForTimeout(1000);
      await page.keyboard.press("Escape");
      await conversationOver(page);
      await page.waitForTimeout(1200);

      await caption(page, "Things on the map can start conversations too — a chest washed up by the tide");
      await interactWith(page, "tide_chest");
      await readToChoices(page, 1400);
      await pick(page, "Close the lid.");
      await conversationOver(page);
      await page.waitForTimeout(1500);
    },
  }),
);
