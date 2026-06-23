import { expect, type Page } from "@playwright/test";

import { center, decode, greenMarker, sceneFraction, type Image } from "./image";

const SCENE_CELLS = 0.3;
// Unoptimized software WebGL is slow to bring up a canvas and spawn the player.
const WORLD_TIMEOUT = 120_000;

let counter = 0;

// Registers a fresh throwaway account by clicking through the sign-in flow, then leaves the redirect
// back to the game for waitForWorld. A unique username per call avoids parallel tests colliding.
export async function register(page: Page): Promise<void> {
  await page.goto("/play");
  await page.getByRole("button", { name: "Sign in to play" }).click();
  await page.getByRole("link", { name: /register/i }).click();

  const user = uniqueUser();
  const password = `Passw0rd-${user}`;
  // Field ids are Keycloak's stable, locale-independent contract.
  await page.locator("#username").fill(user);
  await page.locator("#email").fill(`${user}@example.com`);
  await page.locator("#password").fill(password);
  await page.locator("#password-confirm").fill(password);
  await fillIfPresent(page, "#firstName", user);
  await fillIfPresent(page, "#lastName", user);
  await page.getByRole("button", { name: /register/i }).click();
}

// Waits for the world and the local player's health bar — the click hook can only act once the
// player exists, so tests wait for both.
export async function waitForWorld(page: Page): Promise<void> {
  await canvas(page).waitFor({ state: "visible", timeout: WORLD_TIMEOUT });
  await expect
    .poll(
      async () => {
        const scene = await captureScene(page);
        return sceneFraction(center(scene)) >= SCENE_CELLS && greenMarker(scene) !== null;
      },
      {
        message: "the world and player never appeared (canvas stayed blank or the player never spawned)",
        timeout: WORLD_TIMEOUT,
        intervals: [500],
      },
    )
    .toBe(true);
}

export async function captureScene(page: Page): Promise<Image> {
  return decode(await canvas(page).screenshot());
}

// Clicks a world-space tile via the client's e2e hook (installed on window at startup — see
// game/client/src/testing.rs): a warp tile crosses, any other walks.
export async function clickWorldTile(page: Page, x: number, y: number): Promise<void> {
  await page.evaluate(
    ({ x, y }) => {
      const hook = (window as unknown as { click_world_tile?: (x: number, y: number) => void })
        .click_world_tile;
      if (typeof hook !== "function") throw new Error("window.click_world_tile is not exposed");
      hook(x, y);
    },
    { x, y },
  );
}

// The canvas has no semantic role; target it by id.
function canvas(page: Page) {
  return page.locator("#glcanvas");
}

function uniqueUser(): string {
  counter += 1;
  // Lowercase alphanumerics (Keycloak's username rules), unique across parallel workers and retries.
  return `tester${Date.now().toString(36)}${process.pid.toString(36)}${counter}`;
}

async function fillIfPresent(page: Page, selector: string, value: string): Promise<void> {
  const field = page.locator(selector);
  if ((await field.count()) > 0) {
    await field.fill(value).catch(() => {});
  }
}
