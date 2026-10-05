import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { caption, chapter, enterWorld, inset } from "../helpers/demo";
import { clickUi, dragUi, findUi, focusGame, probe, submitText, waitFor, waitForWorld } from "../helpers/game";
import { loadReference } from "../helpers/image";

let islander: Page;

test(
  "Admin",
  chapter({
    summary: "Admins get a terminal of privileged commands",
    setup: async (page, cast) => {
      islander = await cast.newPage();
      await enterWorld(islander);
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
    },
    play: async (page) => {
      await caption(page, "Admins have a second terminal tab: Admin");
      await focusGame(page);
      await page.keyboard.press("KeyC");
      await clickUi(page, "Admin");
      await page.waitForTimeout(1200);
      await submitText(page, "/commands");
      await page.waitForTimeout(2500);
      await caption(page, "…such as handing out items, like gold");
      await submitText(page, "/give Gold,50");
      await clickUi(page, "icons/equipment/bag.png");
      await page.waitForTimeout(600);
      await dragUi(page, "Inventory", { x: -330, y: -230 });
      await waitFor(page, (snapshot) => findUi(snapshot, "50"), "the gold never arrived");
      await page.waitForTimeout(2000);
      await caption(page, "…or experience");
      await submitText(page, "/xp 120");
      await waitFor(page, (snapshot) => findUi(snapshot, /xp 120$/), "the experience never arrived");
      await page.waitForTimeout(2000);
      await caption(page, "…or teleporting, even to another area");
      await submitText(page, "/tp 20,20,Forest");
      await waitFor(page, ({ area }) => area === "Forest", "the teleport never landed");
      await page.waitForTimeout(3000);
      const name = (await probe(islander)).me!.name;
      const stop = await inset(page, islander, `${name}'s screen, back on the island`);
      try {
        await caption(page, `…or announcing to every player in every area, like ${name} on the island`);
        await submitText(page, "/announce The server restarts in five minutes, so finish your fights.");
        await Promise.all(
          [page, islander].map((player) =>
            waitFor(player, ({ announcement }) => announcement.showing?.by === "Server", "the announcement never arrived"),
          ),
        );
        await page.waitForTimeout(6000);
      } finally {
        await stop();
      }
    },
  }),
);
