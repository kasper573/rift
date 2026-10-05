import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { closestTile, focusGame, nearest, probe, travelTo, waitFor, walkTo } from "../helpers/game";
import { conversationOver, onStage, readToChoices } from "../helpers/talk";

test(
  "Travel",
  chapter({
    summary: "Warps connect the areas of the world",
    play: async (page) => {
      await caption(page, "Warps lead to other areas — click one to cross");
      const { me, portals } = await probe(page);
      const warp = nearest(portals, me!.at)!;
      await travelTo(page, warp.at);
      await onStage(page, "IlsaHalt");
      await caption(page, "This warp is the forest road, and its warden stops first-time travellers", {
        at: "top",
      });
      await readToChoices(page, 1200);
      await page.waitForTimeout(2500);
      await caption(page, "She can't hold you: Esc, and walk on", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("Escape");
      await conversationOver(page);
      await travelTo(page, warp.at);
      await waitFor(page, ({ area }) => area === warp.to, `never arrived in ${warp.to}`);
      await caption(page, `Welcome to the ${warp.to.toLowerCase()}`);
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
