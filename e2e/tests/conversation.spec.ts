import { expect, test } from "@playwright/test";

import { register } from "../helpers/account";
import { clickTile, distance, focusGame, probe, waitFor, waitForWorld, type Snapshot, type Tile } from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, interactWith, talkTo } from "../helpers/talk";

// Clear of the conversation box, which swallows clicks along the bottom of the view.
function openGround(snapshot: Snapshot, from: Tile, height: number): Tile {
  const view = snapshot.view!;
  const candidates = snapshot.walkable.filter((tile) => {
    const y = view.origin[1] + tile[1] * view.tile_size[1];
    return y > 80 && y < height * 0.45 && distance(tile, from) > 3;
  });
  const tile = candidates.at(0);
  if (!tile) throw new Error("no open ground above the conversation box");
  return tile;
}

test("a conversation drops world clicks until Esc leaves it", async ({ page }) => {
  await register(page);
  await waitForWorld(page, loadReference("island.png"));

  const greeting = await talkTo(page, "Grisha");
  expect(greeting.node).toBe("GrishaHello");
  const talking = await waitFor(page, ({ me }) => me?.locked && me, "talking never locked commands");
  const away = openGround(await probe(page), talking.at, page.viewportSize()!.height);

  await clickTile(page, away);
  await page.waitForTimeout(2000);
  expect(distance((await probe(page)).me!.at, talking.at)).toBeLessThan(0.5);

  await focusGame(page);
  await page.keyboard.press("Escape");
  await conversationOver(page);
  await waitFor(page, ({ me }) => me && !me.locked, "leaving never released the lock");

  await clickTile(page, away);
  await waitFor(page, ({ me }) => me && distance(me.at, talking.at) > 1, "a click after leaving never walked");
});

test("a map object opens its own conversation", async ({ page }) => {
  await register(page);
  await waitForWorld(page, loadReference("island.png"));

  const board = await interactWith(page, "HarbourBoard");

  expect(board.node).toBe("HarbourBoard");
  await focusGame(page);
  await page.keyboard.press("Escape");
  await conversationOver(page);
});
