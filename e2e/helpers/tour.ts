import type { Page } from "@playwright/test";

import { provisionAccount, signIn } from "./account";
import { admin } from "./admin";
import {
  clickUi,
  closestTile,
  doubleClickUi,
  focusGame,
  hoverTile,
  nearest,
  occupied,
  probe,
  travelTo,
  waitFor,
  waitForWorld,
  waitUntilStill,
  type Body,
  type Snapshot,
  type Tile,
} from "./game";
import { loadReference } from "./image";

const POTION = "icons/potion/red_potion_3.png";

// The far reaches of the world are no place for a fresh adventurer, so the tour starts seasoned.
export async function arriveSeasoned(page: Page, area: string, at: Tile): Promise<void> {
  await signIn(page, await provisionAccount(page, ["admin"]));
  await clickUi(page, "Play");
  await waitForWorld(page, loadReference("island.png"));
  await admin(page, [
    ["/xp 200", /granted 200 xp/],
    ["/give GreaterHealthPotion,8", /gave 8 \S/],
    [`/tp ${at[0]},${at[1]},${area}`, /teleported/],
  ]);
  await waitFor(page, (snapshot) => snapshot.area === area, `never arrived in ${area}`);
  await waitUntilStill(page);
}

// Heads for the part of the doorway nearest you: a doorway in a room's bottom wall reaches the screen's edge, where
// the HUD sits.
export async function enter(page: Page, portal: string): Promise<void> {
  const door = (await probe(page)).portals.find((exit) => exit.name === portal);
  if (!door) throw new Error(`no portal named ${portal}`);
  const through = (snapshot: Snapshot) => snapshot.area === door.to;
  const {
    origin: [left, top],
    size: [width, height],
  } = door.rect;
  const clamp = (value: number, low: number, high: number) => Math.min(Math.max(value, low), high);
  const doorway = ({ me }: Snapshot): Tile => {
    const at = me?.at ?? door.at;
    return [clamp(at[0], left + 0.5, left + width - 0.5), clamp(at[1], top + 0.5, top + height - 0.5)];
  };
  const deadline = Date.now() + 90_000;
  while (!through(await probe(page))) {
    if (Date.now() > deadline) throw new Error(`never went through ${portal}`);
    await travelTo(page, doorway, 60_000, through).catch(() => undefined);
    await waitFor(page, through, `${portal} never led to ${door.to}`, 6_000).catch(() => undefined);
  }
  await waitUntilStill(page);
}

// Stops just in front of them, where their name plate and face are in plain view, and points them out.
export async function meet(page: Page, npc: string, holdMs = 3000): Promise<Body> {
  const them = (snapshot: Snapshot) => snapshot.actors.find((actor) => actor.npc === npc);
  const front = (snapshot: Snapshot) => {
    const at = them(snapshot)?.at;
    return at && closestTile(snapshot.walkable.filter((tile) => !occupied(snapshot, tile)), [at[0], at[1] + 1.5]);
  };
  await travelTo(page, front);
  await waitUntilStill(page);
  const met = await waitFor(page, them, `${npc} is not here`);
  await hoverTile(page, met.aim);
  await page.waitForTimeout(holdMs);
  return met;
}

export async function walkAlong(page: Page, waypoints: Tile[], pauseMs = 0): Promise<void> {
  for (const point of waypoints) {
    await travelTo(page, (snapshot) => closestTile(snapshot.walkable.filter((tile) => !occupied(snapshot, tile)), point));
    await waitUntilStill(page);
    if (pauseMs) await page.waitForTimeout(pauseMs);
  }
}

// Heads for the middle of a monster region until one shows, then fights the nearest to the death.
export async function hunt(page: Page, npc: string, region: string, kills = 1): Promise<void> {
  const prey = (actor: Body) => actor.npc === npc && actor.health > 0;
  const den = (await probe(page)).markers.find((marker) => marker.name === region)?.at;
  if (!den) throw new Error(`no region named ${region}`);
  const sighted = (snapshot: Snapshot) => snapshot.actors.some(prey);
  const deadline = Date.now() + 180_000;
  for (let slain = 0; slain < kills; ) {
    if (Date.now() > deadline) throw new Error(`slew only ${slain} of ${kills} ${npc}`);
    if (!sighted(await probe(page))) {
      await travelTo(page, den, 60_000, sighted).catch(() => undefined);
      continue;
    }
    if (await slay(page, prey)) slain++;
  }
}

async function slay(page: Page, prey: (actor: Body) => boolean): Promise<boolean> {
  const { me, actors } = await probe(page);
  const foe = me && nearest(actors.filter(prey), me.at);
  if (!foe) return false;
  const live = (snapshot: Snapshot) => snapshot.actors.find((actor) => actor.id === foe.id && prey(actor));
  const deadline = Date.now() + 45_000;
  while (live(await probe(page))) {
    if (Date.now() > deadline) return false;
    await mend(page);
    await travelTo(page, (snapshot) => live(snapshot)?.aim).catch(() => undefined);
    await waitFor(page, (snapshot) => !live(snapshot), "still standing", 5_000).catch(() => undefined);
  }
  await page.waitForTimeout(1500);
  return true;
}

async function mend(page: Page): Promise<void> {
  const { me } = await probe(page);
  if (!me || me.health > me.max_health / 2) return;
  await focusGame(page);
  await page.keyboard.press("KeyI");
  await page.waitForTimeout(500);
  await doubleClickUi(page, POTION).catch(() => undefined);
  await focusGame(page);
  await page.keyboard.press("KeyI");
}
