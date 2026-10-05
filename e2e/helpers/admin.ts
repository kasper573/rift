import type { Page } from "@playwright/test";

import { clickUi, findUi, focusGame, waitFor } from "./game";

export async function give(page: Page, items: [string, number][]): Promise<void> {
  await focusGame(page);
  await page.keyboard.press("KeyC");
  await clickUi(page, "Admin");
  await page.waitForTimeout(1200);
  for (const [item, count] of items) {
    await clickUi(page, (element) => element.editable);
    await page.waitForTimeout(400);
    await page.keyboard.type(`/give ${item},${count}`, { delay: 45 });
    await page.keyboard.press("Enter");
    await waitFor(page, (snapshot) => findUi(snapshot, new RegExp(`gave ${count} \\S`)), `${item} never arrived`);
  }
  await focusGame(page);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);
  await page.keyboard.press("Escape");
  await waitFor(page, (snapshot) => !findUi(snapshot, "Admin"), "the terminal never closed");
}
