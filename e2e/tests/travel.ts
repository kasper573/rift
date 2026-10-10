import type { Page } from "@playwright/test";

import {
  canvasSize,
  clickTile,
  closestTile,
  distance,
  leaveConversation,
  occupied,
  probe,
  type Cover,
  type Snapshot,
  type Tile,
  type TileRect,
} from "../helpers/game";

// The suite must hold on any screen, down to the smallest, where few tiles fit and the interface covers
// much of them, and on software rendering, where frames come slowly. So it walks for precision over
// pace: every click is aimed from rest, at a spot exactly on screen and clear of the interface.

// Only on-screen tiles can be clicked, so a far target is approached hop by hop, each one nearer. A
// function target is re-read every hop, for things that move. While the player walks, the ground under
// the cursor slides, and a click lands short of where it aimed.
export async function travel(
  page: Page,
  target: Tile | ((snapshot: Snapshot) => Tile | undefined),
  timeout = 60_000,
  arrived: (snapshot: Snapshot) => unknown = () => false,
): Promise<void> {
  const locate = typeof target === "function" ? target : () => target;
  const deadline = Date.now() + timeout;
  for (;;) {
    await waitAtRest(page);
    const snapshot = await probe(page);
    if (arrived(snapshot)) return;
    if (snapshot.stage) {
      await leaveConversation(page);
      continue;
    }
    const tile = locate(snapshot);
    if (!tile) throw new Error("the travel target is gone");
    if (clickable(snapshot, tile, await canvasSize(page))) {
      await clickTile(page, tile);
      return;
    }
    if (Date.now() > deadline) throw new Error(`never got ${tile} on screen`);
    const me = snapshot.me?.at;
    const nearer = (step: Tile) => !me || distance(step, tile) < distance(me, tile) - 0.5;
    const ground = await openGround(page, snapshot);
    const hop = closestTile(ground.filter((step) => nearer(step) && !inWarp(snapshot, step)), tile);
    if (!hop) throw new Error(`no open ground on screen leads toward ${tile}`);
    await clickTile(page, hop);
  }
}

// Where a click walks to: ground on screen, clear of the interface, with no one standing on it.
export async function openGround(page: Page, snapshot: Snapshot): Promise<Tile[]> {
  const size = await canvasSize(page);
  return snapshot.walkable.filter((tile) => clickable(snapshot, tile, size) && !occupied(snapshot, tile));
}

// At rest, the camera has caught up too: it glides after the player, so the ground under the cursor
// keeps shifting a moment after they stop.
export async function waitAtRest(page: Page, timeout = 30_000): Promise<Tile> {
  const deadline = Date.now() + timeout;
  let last: string | undefined;
  let at: Tile | undefined;
  let still = 0;
  await page.waitForTimeout(400);
  while (still < 3) {
    if (Date.now() > deadline) throw new Error("the player never came to rest");
    const snapshot = await probe(page);
    const pose = JSON.stringify([snapshot.me?.at, snapshot.view?.origin]);
    still = snapshot.me && pose === last ? still + 1 : 0;
    last = pose;
    at = snapshot.me?.at;
    await page.waitForTimeout(250);
  }
  return at!;
}

function clickable(snapshot: Snapshot, tile: Tile, size: { width: number; height: number }): boolean {
  if (!snapshot.view) return false;
  const x = snapshot.view.origin[0] + tile[0] * snapshot.view.tile_size[0];
  const y = snapshot.view.origin[1] + tile[1] * snapshot.view.tile_size[1];
  const under = (cover: Cover) =>
    x >= cover.x && y >= cover.y && x <= cover.x + cover.width && y <= cover.y + cover.height;
  return x > 0 && y > 0 && x < size.width && y < size.height && !snapshot.covered.some(under);
}

// A click inside a warp takes it; walking across one on the way elsewhere doesn't.
function inWarp(snapshot: Snapshot, tile: Tile): boolean {
  return snapshot.portals.some((portal) => within(portal.rect, tile));
}

function within({ origin, size }: TileRect, at: Tile): boolean {
  return at[0] >= origin[0] && at[1] >= origin[1] && at[0] < origin[0] + size[0] && at[1] < origin[1] + size[1];
}
