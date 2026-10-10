import { expect, test, type Page } from "@playwright/test";

import { register } from "../helpers/account";
import { clickTile, distance, focusGame, probe, waitFor, waitForWorld, type Tile } from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, interactWith, talkTo } from "../helpers/talk";
import { openGround, travel, waitAtRest } from "./travel";

// Townsfolk stroll, and a click on one talks instead of walking: the open ground farthest from anyone
// stays open until the click lands.
async function openGroundAway(page: Page, from: Tile): Promise<Tile> {
  const snapshot = await probe(page);
  const others = snapshot.actors.filter((actor) => actor.id !== snapshot.me?.id).map((actor) => actor.at);
  const room = (tile: Tile) => Math.min(...others.map((at) => distance(at, tile)));
  const ground = (await openGround(page, snapshot)).filter((tile) => distance(tile, from) >= 2);
  const tile = ground.sort((a, b) => room(b) - room(a)).at(0);
  if (!tile) throw new Error("no open ground beside the conversation");
  return tile;
}

test("a conversation drops world clicks until Esc leaves it", async ({ page }) => {
  await register(page);
  await waitForWorld(page, loadReference("island.png"));

  const greeting = await talkTo(page, "Grisha", travel);
  expect(greeting.node).toBe("GrishaHello");
  const talking = await waitFor(page, ({ me }) => me?.locked && me, "talking never locked commands");
  await waitAtRest(page);

  await clickTile(page, await openGroundAway(page, talking.at));
  await page.waitForTimeout(2000);
  expect(distance((await probe(page)).me!.at, talking.at)).toBeLessThan(0.5);

  await focusGame(page);
  await page.keyboard.press("Escape");
  await conversationOver(page);
  await waitFor(page, ({ me }) => me && !me.locked, "leaving never released the lock");

  await clickTile(page, await openGroundAway(page, talking.at));
  await waitFor(page, ({ me }) => me && distance(me.at, talking.at) > 1, "a click after leaving never walked");
});

test("a map object opens its own conversation", async ({ page }) => {
  await register(page);
  await waitForWorld(page, loadReference("island.png"));

  const board = await interactWith(page, "HarbourBoard", travel);

  expect(board.node).toBe("HarbourBoard");
  await focusGame(page);
  await page.keyboard.press("Escape");
  await conversationOver(page);
});
