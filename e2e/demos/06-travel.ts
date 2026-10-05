import { test } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { admin } from "../helpers/admin";
import { caption, chapter } from "../helpers/demo";
import {
  clickUi,
  closestTile,
  focusGame,
  holding,
  hoverUi,
  onChoice,
  probe,
  rightClickUi,
  travelTo,
  waitFor,
  waitForWorld,
  walkTo,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, onStage, pick, readToChoices } from "../helpers/talk";

const PASS = "icons/misc/scroll.png";

test(
  "Travel",
  chapter({
    summary: "Warps connect the areas of the world, and some are locked",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await admin(page, [["/give Gold,20", /gave 20 Gold/]]);
    },
    play: async (page) => {
      await caption(page, "Warps lead to other areas — click one to cross");
      const road = (await probe(page)).portals.find((portal) => portal.name === "forest-road")!;
      await travelTo(page, road.at);
      await onStage(page, "IlsaHalt");
      await caption(page, "The forest road is locked without a pass: you just stand on it, and its warden calls out", {
        at: "top",
      });
      await readToChoices(page, 1200);
      await page.waitForTimeout(2000);
      await caption(page, "Twenty Gold buys a pass", { at: "top" });
      await hoverUi(page, "Here, for your trouble.");
      await page.waitForTimeout(1500);
      await caption(page, "Hover what a choice gives you for a glimpse of it — right-click for its card", { at: "top" });
      await hoverUi(page, onChoice(await probe(page), PASS));
      await page.waitForTimeout(2500);
      await rightClickUi(page, onChoice(await probe(page), PASS));
      await waitFor(page, ({ item_card }) => item_card?.item === "RoadPass", "the pass's card never opened");
      await page.waitForTimeout(4000);
      await caption(page, "Esc closes the card, and the conversation carries on", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("Escape");
      await waitFor(page, ({ item_card, stage }) => item_card === null && stage !== null, "the card never closed");
      await page.waitForTimeout(1200);
      await pick(page, "Here, for your trouble.");
      await onStage(page, "IlsaBribed");
      await readToChoices(page, 1200);
      await pick(page, "Goodbye.");
      await conversationOver(page);
      await waitFor(page, (snapshot) => holding(snapshot, "RoadPass") === 1, "the pass never arrived");
      await caption(page, "With the pass in your bag, the same warp takes you through");
      await travelTo(page, road.at);
      await waitFor(page, ({ area }) => area === road.to, `never arrived in ${road.to}`);
      await caption(page, `Welcome to the ${road.to.toLowerCase()}`);
      for (const [dx, dy] of [
        [3, 2],
        [-2, 3],
      ]) {
        const { me, walkable } = await probe(page);
        await walkTo(page, closestTile(walkable, [me!.at[0] + dx, me!.at[1] + dy])!);
      }
      await page.waitForTimeout(1500);
    },
  }),
);
