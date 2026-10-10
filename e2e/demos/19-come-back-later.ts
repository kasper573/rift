import { test } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { give } from "../helpers/admin";
import { caption, chapter } from "../helpers/demo";
import { clickUi, finished, focusGame, historyTab, onQuest, waitFor, waitForWorld } from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, onStage, pick, readToChoices, talkTo } from "../helpers/talk";

test(
  "Come back later",
  chapter({
    summary: "Wren's daily tithe, and an answer that knows what time it is",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await give(page, [["bone", 10]]);
    },
    play: async (page) => {
      await caption(page, "Wren takes ten bones a day, and pays in tokens");
      await talkTo(page, "wren");
      await readToChoices(page, 900);
      await pick(page, "Bone Tithe");
      await onStage(page, "tithe_offer");
      await readToChoices(page, 1400);
      await pick(page, "I'll bring today's bones.");
      await waitFor(page, (snapshot) => onQuest(snapshot, "bone_tithe")?.ready, "the tithe never got ready");
      await caption(page, "Ten bones already in the bag: the tithe is ready at once", { at: "top" });
      await page.waitForTimeout(2500);

      await talkTo(page, "wren");
      await readToChoices(page, 900);
      await pick(page, "Bone Tithe");
      await onStage(page, "tithe_thanks");
      await readToChoices(page, 1200);
      await caption(page, "She named her price and her pay, so the hand-in shows both", { at: "top" });
      await page.waitForTimeout(2500);
      await pick(page, "Here are today's bones.");
      await waitFor(page, (snapshot) => finished(snapshot, "bone_tithe") === "Completed", "the tithe was never paid");
      await waitFor(page, ({ feed }) => feed.includes("+6 Bone Token"), "the tokens never showed in the feed");
      await caption(page, "Bones out, tokens in, and the day's tithe is done", { at: "top" });
      await page.waitForTimeout(3000);

      await caption(page, "Once it's done for the day, she answers a different question");
      await talkTo(page, "wren");
      await readToChoices(page, 900);
      await page.waitForTimeout(1500);
      await pick(page, "When can I bring more bones?");
      await onStage(page, "tithe_tomorrow");
      await waitFor(page, ({ stage }) => stage?.text?.includes("Come back in") && !stage.typing, "she never said when");
      await caption(page, "She works out the time left as she says it, in the words anyone would use", { at: "top" });
      await page.waitForTimeout(3500);
      await readToChoices(page, 1400);
      await pick(page, "I'll be back.");
      await conversationOver(page);

      await caption(page, "History keeps what she said, exactly as she said it", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("KeyH");
      await waitFor(page, ({ history }) => history.open, "history never opened");
      const talk = await historyTab(page, "Talk");
      await clickUi(page, (element) => element.text === talk.text && element.y === talk.y && element.x === talk.x);
      await waitFor(
        page,
        ({ history }) => history.tab === "Talk" && history.records.some((record) => record.text.includes("Come back in")),
        "her answer never reached history",
      );
      await page.waitForTimeout(4000);
      await page.keyboard.press("Escape");
      await waitFor(page, ({ history }) => !history.open, "history never closed");
    },
  }),
);
