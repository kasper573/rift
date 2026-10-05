import type { Page } from "@playwright/test";

import { clickUi, findUi, focusGame, waitFor } from "./game";

export async function give(page: Page, items: [string, number][]): Promise<void> {
  await admin(
    page,
    items.map(([item, count]) => [`/give ${item},${count}`, new RegExp(`gave ${count} \\S`)]),
  );
}

export async function admin(page: Page, commands: [string, RegExp][]): Promise<void> {
  await focusGame(page);
  await page.keyboard.press("KeyC");
  await clickUi(page, "Admin");
  await page.waitForTimeout(1200);
  for (const [command, reply] of commands) {
    await clickUi(page, (element) => element.editable);
    await page.waitForTimeout(400);
    await page.keyboard.type(command, { delay: 45 });
    await page.keyboard.press("Enter");
    await waitFor(page, (snapshot) => findUi(snapshot, reply), `${command} never answered`);
  }
  await focusGame(page);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);
  await page.keyboard.press("Escape");
  await waitFor(page, (snapshot) => !findUi(snapshot, "Admin"), "the terminal never closed");
}
