import { test, type Page } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import {
  closestTile,
  distance,
  doubleClickUi,
  focusGame,
  nearest,
  occupied,
  pickUp,
  probe,
  safe,
  travelTo,
  waitFor,
  walkTo,
  wayOut,
  type Body,
  type GroundItem,
  type Snapshot,
  type Tile,
} from "../helpers/game";

test(
  "Combat",
  chapter({
    summary: "Fight monsters, loot what they drop, grow stronger and run for town",
    play: async (page) => {
      await caption(page, "Town is a safe zone: monsters roam the wilds beyond it");
      const footsteps = track(page);
      await headOut(page);
      await caption(page, "Hover a monster to target it, click to attack");
      let loot: GroundItem[] = [];
      for (let attempt = 0; attempt < 4 && loot.length === 0; attempt++) {
        loot = await slay(page);
      }
      if (loot.length === 0) throw new Error("no monster fell and dropped loot");
      await caption(page, "Monsters drop loot — click it to pick it up");
      const carried = (snapshot: Snapshot) => snapshot.bag.reduce((total, stack) => total + stack.count, 0);
      const before = carried(await probe(page));
      // Loot lying under a monster can't be clicked until it moves or falls, so some may stay behind.
      for (const item of loot) {
        await pickUp(page, item).catch(() => undefined);
      }
      if (carried(await probe(page)) === before) throw new Error("picked up none of the loot");
      await caption(page, "Press I to open the inventory");
      await focusGame(page);
      await page.keyboard.press("KeyI");
      await page.waitForTimeout(2000);
      const { ui } = await probe(page);
      const potion = ui.find((element) => element.image?.includes("potion"));
      if (potion) {
        await caption(page, "Double-click a potion to drink it");
        await doubleClickUi(page, potion.image!);
        await page.waitForTimeout(1500);
      }
      await caption(page, "Kills earn experience — see the xp under your name");
      await page.waitForTimeout(3000);
      await focusGame(page);
      if ((await probe(page)).item_card) await page.keyboard.press("Escape");
      await page.keyboard.press("KeyI");

      await caption(page, "Pick a fight, then run for town");
      const chaser = await provoke(page);
      await footsteps.stop();
      await runBack(page, footsteps.trail);
      await caption(page, "It gives up the chase at the edge: monsters never set foot in a safe zone");
      await watchFromSafety(page, chaser, 6000);
    },
  }),
);

async function headOut(page: Page): Promise<void> {
  const deadline = Date.now() + 90_000;
  while (!(await probe(page)).actors.some(isPrey)) {
    if (Date.now() > deadline) throw new Error("never met a monster outside town");
    await travelTo(page, (snapshot) => wayOut(snapshot) ?? snapshot.me?.at).catch(() => undefined);
    await page.waitForTimeout(1000);
  }
}

async function slay(page: Page): Promise<GroundItem[]> {
  const foe = await waitFor(page, ({ me, actors }) => me && nearest(actors.filter(isPrey), me.at), "no monster to fight");
  const live = (actors: Body[]) => actors.find((actor) => actor.id === foe.id && isPrey(actor));
  let fell = foe.at;
  const fallen = ({ actors }: Snapshot) => {
    const it = live(actors);
    if (it) fell = it.at;
    return !it;
  };
  // In a crowd, whatever flies over the monster takes the click, so a fight that drags on is given up.
  const deadline = Date.now() + 45_000;
  while (!fallen(await probe(page))) {
    if (Date.now() > deadline) return [];
    // A moving monster can slip out from under the click, so it is clicked again until it falls.
    await travelTo(page, ({ actors }) => live(actors)?.aim).catch(() => undefined);
    await waitFor(page, fallen, "the monster is still standing", 8_000).catch(() => undefined);
  }
  await page.waitForTimeout(1500);
  return (await probe(page)).items.filter((item) => distance(item.at, fell) < 2.5);
}

async function provoke(page: Page): Promise<Body> {
  const deadline = Date.now() + 60_000;
  for (;;) {
    if (Date.now() > deadline) throw new Error("no monster took up the fight");
    const snapshot = await probe(page);
    if (!snapshot.actors.some(isPrey)) {
      await travelTo(page, (seen) => wayOut(seen) ?? seen.me?.at).catch(() => undefined);
      continue;
    }
    const foe = nearest(snapshot.actors.filter(isPrey), snapshot.me!.at)!;
    await travelTo(page, ({ actors }) => actors.find((actor) => actor.id === foe.id && isPrey(actor))?.aim).catch(
      () => undefined,
    );
    const engaged = await waitFor(
      page,
      ({ me, actors }) => {
        const it = actors.find((actor) => actor.id === foe.id && isPrey(actor));
        return me && it && distance(me.at, it.at) < 2 ? it : undefined;
      },
      "the monster never came to blows",
      8_000,
    ).catch(() => undefined);
    if (engaged) return engaged;
  }
}

// Retraces the trail a few tiles at a time: every step of it was walked once, so it leads back to
// town even where water lies between.
async function runBack(page: Page, trail: Tile[]): Promise<void> {
  const deadline = Date.now() + 90_000;
  for (;;) {
    const snapshot = await probe(page);
    const me = snapshot.me!.at;
    if (safe(snapshot, me)) return;
    if (Date.now() > deadline) throw new Error("never made it back to town");
    const near = trail.findIndex((at) => distance(at, me) < 2);
    let back = near >= 0 ? near : trail.indexOf(closestTile(trail, me)!);
    while (back > 0 && distance(trail[back], me) < 5) back--;
    const step = closestTile(
      snapshot.walkable.filter((tile) => !occupied(snapshot, tile)),
      trail[back],
    );
    if (step) await walkTo(page, step).catch(() => undefined);
  }
}

// Where the player has been, oldest first.
function track(page: Page): { trail: Tile[]; stop: () => Promise<void> } {
  const trail: Tile[] = [];
  let tracking = true;
  const done = (async () => {
    while (tracking && !page.isClosed()) {
      const at = (await probe(page)).me?.at;
      if (at && (trail.length === 0 || distance(trail.at(-1)!, at) >= 1)) trail.push(at);
      await page.waitForTimeout(300);
    }
  })().catch(() => undefined);
  return {
    trail,
    stop: async () => {
      tracking = false;
      await done;
    },
  };
}

async function watchFromSafety(page: Page, chaser: Body, forMs: number): Promise<void> {
  const until = Date.now() + forMs;
  while (Date.now() < until) {
    const snapshot = await probe(page);
    const it = snapshot.actors.find((actor) => actor.id === chaser.id);
    if (it && safe(snapshot, it.at)) throw new Error(`${it.name} followed you into town`);
    await page.waitForTimeout(250);
  }
}

// Bats are left alone: they fly over water, where nobody on foot can follow.
function isPrey(actor: Body): boolean {
  return !actor.player && !actor.friendly && !actor.flies && actor.health > 0;
}
