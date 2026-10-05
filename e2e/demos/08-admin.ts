import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { caption, chapter } from "../helpers/demo";
import { clickUi, dragUi, findUi, focusGame, waitFor, waitForWorld } from "../helpers/game";
import { loadReference } from "../helpers/image";

test(
  "Admin",
  chapter({
    summary: "Admins get a terminal of privileged commands",
    setup: async (page) => {
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
      await command(page, "/commands");
      await page.waitForTimeout(2500);
      await caption(page, "…such as handing out items, like gold");
      await command(page, "/give Gold,50");
      await clickUi(page, "icons/equipment/bag.png");
      await page.waitForTimeout(600);
      await dragUi(page, "Inventory", { x: -330, y: -230 });
      await waitFor(page, (snapshot) => findUi(snapshot, "50"), "the gold never arrived");
      await page.waitForTimeout(2000);
      await caption(page, "…or experience");
      await command(page, "/xp 120");
      await waitFor(page, (snapshot) => findUi(snapshot, /xp 120$/), "the experience never arrived");
      await page.waitForTimeout(2000);
      await caption(page, "…or teleporting, even to another area");
      await command(page, "/tp 20,20,Forest");
      await waitFor(page, ({ area }) => area === "Forest", "the teleport never landed");
      await page.waitForTimeout(3000);
    },
  }),
);

async function command(page: Page, text: string): Promise<void> {
  await clickUi(page, (element) => element.editable);
  await page.keyboard.type(text, { delay: 45 });
  await page.keyboard.press("Enter");
}
