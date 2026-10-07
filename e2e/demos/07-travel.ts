import { test } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { admin } from "../helpers/admin";
import { caption, chapter, soundscapeMixer } from "../helpers/demo";
import {
  clickUi,
  closestTile,
  holding,
  hoverUi,
  probe,
  travelTo,
  waitFor,
  waitForWorld,
  walkTo,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, onStage, pick, readToChoices } from "../helpers/talk";

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
      await pick(page, "Here, for your trouble.");
      await onStage(page, "IlsaBribed");
      await readToChoices(page, 1200);
      await pick(page, "Goodbye.");
      await conversationOver(page);
      await waitFor(page, (snapshot) => holding(snapshot, "RoadPass") === 1, "the pass never arrived");
      await caption(page, "With the pass in your bag, the same warp takes you through");
      const hideMixer = await soundscapeMixer(page);
      await travelTo(page, road.at);
      await waitFor(page, ({ area }) => area === road.to, `never arrived in ${road.to}`);
      await caption(page, `Welcome to the ${road.to.toLowerCase()}: its own music and birdsong crossfade in over the harbour's`);
      await page.waitForTimeout(5000);
      await caption(page, "A camp's fire carries past its edge, louder the closer you walk");
      for (const [dx, dy] of [
        [3, -2],
        [2, -1],
      ]) {
        const { me, walkable } = await probe(page);
        await walkTo(page, closestTile(walkable, [me!.at[0] + dx, me!.at[1] + dy])!);
        await page.waitForTimeout(2500);
      }
      await page.waitForTimeout(1500);
      await hideMixer();
    },
  }),
);
