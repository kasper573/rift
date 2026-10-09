import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { admin } from "../helpers/admin";
import { caption, chapter, soundscapeMixer } from "../helpers/demo";
import {
  arrivedIn,
  clickUi,
  closestTile,
  findUi,
  focusGame,
  holding,
  hoverUi,
  nearest,
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
    summary: "Warps connect the areas of the world, some are locked, and each crossing plays a transition",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await admin(page, [["/give Gold,20", /gave 20 Gold/]]);
    },
    play: async (page) => {
      await caption(page, "Warps lead to other areas — click one to cross");
      const { area: home, portals } = await probe(page);
      const road = portals.find((portal) => portal.name === "forest-road")!;
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
      await caption(page, "With the pass in your bag, the same warp takes you through: the island crumbles away, and the forest rises around you");
      const hideMixer = await soundscapeMixer(page);
      await travelTo(page, road.at);
      await waitFor(page, (snapshot) => arrivedIn(snapshot, road.to), `never arrived in ${road.to}`);
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

      await caption(page, "Settings choose how a crossing looks", { at: "top" });
      let current = "Crumble";
      let there = road.to;
      for (const [style, says] of TRANSITIONS) {
        await chooseTransition(page, current, style);
        current = style;
        there = there === road.to ? home! : road.to;
        await caption(page, says);
        await crossInto(page, there);
        await page.waitForTimeout(1200);
      }
    },
  }),
);

const TRANSITIONS: [string, string][] = [
  ["Tile wave", "Tile wave: the tiles shrink to diamonds around you, and grow back where you land"],
  ["Sweep", "Sweep: bands carry the old area off the way you walk, and bring the next in behind them"],
  ["Iris", "Iris: a spotlight closes on you, and opens again where you arrive"],
  ["Mosaic", "Mosaic: the area coarsens into blocks, steps over to the next, and resolves"],
  ["Dither dissolve", "Dither dissolve: the area fizzles out pixel by pixel, and the next fizzles in around you"],
  ["Fade", "Fade: plain and quick"],
];

async function chooseTransition(page: Page, current: string, next: string): Promise<void> {
  await focusGame(page);
  await page.keyboard.press("KeyO");
  await hoverUi(page, /^reduced motion/);
  await page.mouse.wheel(0, -1000);
  await page.waitForTimeout(600);
  const settings = await probe(page);
  const motion = findUi(settings, /^reduced motion/)!;
  const picker = findUi(settings, current)!;
  await page.mouse.wheel(0, picker.y + picker.height - (motion.y + motion.height));
  await page.waitForTimeout(600);
  await clickUi(page, current);
  await hoverUi(page, next);
  await page.waitForTimeout(700);
  await clickUi(page, next);
  await waitFor(page, (snapshot) => findUi(snapshot, next) && !findUi(snapshot, current), `${next} was never picked`);
  await page.waitForTimeout(500);
  await focusGame(page);
  await page.keyboard.press("KeyO");
  await waitFor(page, (snapshot) => !findUi(snapshot, next), "the settings never closed");
}

async function crossInto(page: Page, area: string): Promise<void> {
  const { me, portals } = await probe(page);
  const warp = nearest(portals.filter((portal) => portal.to === area), me!.at)!;
  await travelTo(page, warp.at);
  await waitFor(page, (snapshot) => arrivedIn(snapshot, area), `never crossed into ${area}`);
}
