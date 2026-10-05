import type { Page } from "@playwright/test";

import {
  clickUi,
  focusGame,
  probe,
  travelTo,
  waitFor,
  type Body,
  type Fixture,
  type Shop,
  type Snapshot,
  type Stage,
  type Tile,
} from "./game";

export function townsperson(snapshot: Snapshot, npc: string): Body | undefined {
  return snapshot.actors.find((actor) => actor.npc === npc);
}

export function fixture(snapshot: Snapshot, prop: string): Fixture | undefined {
  return snapshot.props.find((candidate) => candidate.prop === prop);
}

export async function talkTo(page: Page, npc: string): Promise<Stage> {
  return reachUntil(
    page,
    (snapshot) => townsperson(snapshot, npc)?.aim,
    (snapshot) => snapshot.stage?.with === townsperson(snapshot, npc)?.id && snapshot.stage,
    `the conversation with ${npc} never opened`,
  );
}

export async function interactWith(page: Page, prop: string): Promise<Stage> {
  return reachUntil(
    page,
    (snapshot) => fixture(snapshot, prop)?.aim,
    (snapshot) => snapshot.stage?.with === fixture(snapshot, prop)?.id && snapshot.stage,
    `interacting with ${prop} never opened a conversation`,
  );
}

export async function shopAt(page: Page, prop: string): Promise<Shop> {
  return reachUntil(
    page,
    (snapshot) => fixture(snapshot, prop)?.aim,
    ({ shop }) => shop,
    `using ${prop} never opened a shop`,
  );
}

export async function lineRead(page: Page): Promise<Stage> {
  return waitFor(page, ({ stage }) => stage && !stage.typing && stage, "the line never finished typing");
}

export async function readToChoices(page: Page, pauseMs = 900): Promise<Stage> {
  await focusGame(page);
  for (;;) {
    const stage = await lineRead(page);
    if (stage.line + 1 >= stage.lines) return stage;
    await page.waitForTimeout(pauseMs);
    await page.keyboard.press("Space");
    await waitFor(page, (snapshot) => snapshot.stage?.line !== stage.line, "the line never advanced");
  }
}

// The same words can show elsewhere on screen (a quest title on the tracker), so only the choice row counts.
export async function pick(page: Page, label: string): Promise<void> {
  const row = await waitFor(
    page,
    ({ stage }) => stage?.choices.find((choice) => choice.label === label)?.rect,
    `no choice ${label}`,
  );
  await clickUi(
    page,
    (element) =>
      element.text === label &&
      element.y >= row.y &&
      element.y + element.height <= row.y + row.height + 1 &&
      element.x >= row.x &&
      element.x <= row.x + row.width,
  );
}

export async function conversationOver(page: Page): Promise<void> {
  await waitFor(page, ({ stage }) => stage === null, "the conversation never ended");
}

export async function onStage(page: Page, node: string): Promise<Stage> {
  return waitFor(page, ({ stage }) => stage?.node === node && stage, `${node} never came on stage`);
}

export async function stageNow(page: Page): Promise<Stage | null> {
  return (await probe(page)).stage;
}

// Townsfolk stroll, so a click can land where one stood a moment ago; click again, as a player would.
async function reachUntil<T>(
  page: Page,
  aim: (snapshot: Snapshot) => Tile | undefined,
  done: (snapshot: Snapshot) => T | null | undefined | false,
  message: string,
): Promise<T> {
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    await travelTo(page, aim, 90_000);
    const settle = Date.now() + 8_000;
    while (Date.now() < settle) {
      const snapshot = await probe(page);
      const found = done(snapshot);
      if (found) return found;
      if (snapshot.stage) break;
      await page.waitForTimeout(100);
    }
  }
  throw new Error(message);
}
