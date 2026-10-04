import { test, type Page } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import {
  clickUi,
  distance,
  focusGame,
  nearest,
  pickUp,
  probe,
  travelTo,
  waitFor,
  type Body,
  type GroundItem,
} from "../helpers/game";

test(
  "Combat",
  chapter({
    summary: "Fight monsters, loot what they drop and grow stronger",
    play: async (page) => {
      await caption(page, "Hover a monster to target it, click to attack");
      let loot: GroundItem[] = [];
      for (let attempt = 0; attempt < 4 && loot.length === 0; attempt++) {
        loot = await slay(page);
      }
      await caption(page, "Monsters drop loot — click it to pick it up");
      for (const item of loot) {
        await pickUp(page, item);
      }
      await caption(page, "Press I to open the inventory");
      await focusGame(page);
      await page.keyboard.press("KeyI");
      await page.waitForTimeout(2000);
      const { ui } = await probe(page);
      const potion = ui.find((element) => element.image?.includes("potion"));
      if (potion) {
        await caption(page, "Click a potion to drink it");
        await clickUi(page, potion.image!);
        await page.waitForTimeout(1500);
      }
      await caption(page, "Kills earn experience — see the xp under your name");
      await page.waitForTimeout(3000);
    },
  }),
);

async function slay(page: Page): Promise<GroundItem[]> {
  const foe = await waitFor(
    page,
    ({ me, actors }) => me && nearest(actors.filter(isAliveMonster), me.at),
    "no monster to fight",
  );
  const live = (actors: Body[]) => actors.find((actor) => actor.id === foe.id && isAliveMonster(actor));
  await travelTo(page, ({ actors }) => live(actors)?.aim);
  let fell = foe.at;
  await waitFor(
    page,
    ({ actors }) => {
      const it = live(actors);
      if (it) fell = it.at;
      return !it;
    },
    "the monster never fell",
    60_000,
  );
  await page.waitForTimeout(1500);
  return (await probe(page)).items.filter((item) => distance(item.at, fell) < 2.5);
}

function isAliveMonster(actor: Body): boolean {
  return !actor.player && actor.health > 0;
}
