import { expect, type Page } from "@playwright/test";

import { center, decode, greenMarker, sceneFraction, type Image } from "./image";

// The game draws every tile at a fixed 48 CSS px on every screen (render.rs), and the health bar
// sits 5 world px (×3) below the player's position.
const TILE_PX = 48;
const BAR_DROP_PX = 15;

// A click holds the button down this long. The client samples the mouse button once per rendered
// frame, so press and release must land in different frames for it to register a click — on software
// WebGL under load (large canvases, headed Firefox) frames can be hundreds of ms apart, so the hold
// is generous enough to span several. This is an input duration, not a wait for app state.
const CLICK_HOLD = 1500;

// A rendered map lights up at least this fraction of the view; below it the canvas is still blank or
// loading. The timeout is generous: wasm init, the netcode connect, and the first SwiftShader frame
// are slow.
const SCENE_CELLS = 0.3;
const WORLD_TIMEOUT = 120_000;

let counter = 0;

// Registers a fresh throwaway account and lands signed-in on /play. From the signed-out page, follow
// the sign-in button to Keycloak, jump straight to its registration form, fill it, and submit;
// Keycloak redirects back through the website callback to /play. A unique username per call keeps
// parallel tests from colliding on the one shared Keycloak.
export async function register(page: Page): Promise<void> {
  await page.goto("/play");
  await page.getByRole("button", { name: "Sign in to play" }).click();

  await page.waitForURL(/\/protocol\/openid-connect\/auth/, { timeout: 60_000 });
  const registrationUrl = page
    .url()
    .replace("/protocol/openid-connect/auth?", "/protocol/openid-connect/registrations?");
  await page.goto(registrationUrl);

  const user = uniqueUser();
  const password = `Passw0rd-${user}`;
  // Keycloak's field ids are a stable, locale-independent contract — steadier here than its
  // localized field labels.
  await page.locator("#username").fill(user);
  await page.locator("#email").fill(`${user}@example.com`);
  await page.locator("#password").fill(password);
  await page.locator("#password-confirm").fill(password);
  await fillIfPresent(page, "#firstName", user);
  await fillIfPresent(page, "#lastName", user);
  await page.getByRole("button", { name: /register/i }).click();

  await page.waitForURL(/\/play(?:[?#]|$)/, { timeout: 60_000 });
}

// Waits until the game canvas is showing the world, not a blank or loading frame.
export async function waitForWorld(page: Page): Promise<void> {
  await canvas(page).waitFor({ state: "visible", timeout: WORLD_TIMEOUT });
  await expect
    .poll(async () => sceneFraction(center(await captureScene(page))), {
      message: "the game world never appeared (the canvas stayed blank)",
      timeout: WORLD_TIMEOUT,
      intervals: [500],
    })
    .toBeGreaterThanOrEqual(SCENE_CELLS);
}

export async function captureScene(page: Page): Promise<Image> {
  return decode(await canvas(page).screenshot());
}

// Clicks a tile offset from the player (east = +x, north = +y). Locates the player by its health bar
// and clicks the given number of tiles away — well-known coordinates relative to the character,
// robust to where the view happens to place it. The camera centers the player, so these tiles are
// always in view for small offsets.
export async function clickFromPlayer(page: Page, tilesEast: number, tilesNorth: number): Promise<void> {
  const box = await canvasBox(page);
  const scene = await captureScene(page);
  const marker = greenMarker(scene);
  if (!marker) throw new Error("could not find the local player's health bar on the canvas");
  // The screenshot is in backing pixels; map to the element's CSS pixels, where clicks are aimed.
  const scaleX = box.width / scene.width;
  const scaleY = box.height / scene.height;
  const playerX = marker.x * scaleX;
  const playerY = marker.y * scaleY - BAR_DROP_PX;
  await press(page, box.x + playerX + tilesEast * TILE_PX, box.y + playerY - tilesNorth * TILE_PX);
}

// Presses and holds the button at a viewport point long enough for the client to sample it. The hold
// outlasts a frame; an instant press+release can fall between frames on the software renderer. This
// is an input duration, not a wait for app state.
async function press(page: Page, x: number, y: number): Promise<void> {
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.waitForTimeout(CLICK_HOLD);
  await page.mouse.up();
}

async function canvasBox(page: Page) {
  const box = await canvas(page).boundingBox();
  if (!box) throw new Error("the game canvas has no bounding box");
  return box;
}

// The game canvas has no semantic role, so target it by its stable id.
function canvas(page: Page) {
  return page.locator("#glcanvas");
}

function uniqueUser(): string {
  counter += 1;
  // Base36 of the clock, the worker process, and a per-worker counter — lowercase alphanumerics that
  // satisfy Keycloak's username rules and stay unique across parallel workers and retries.
  return `tester${Date.now().toString(36)}${process.pid.toString(36)}${counter}`;
}

async function fillIfPresent(page: Page, selector: string, value: string): Promise<void> {
  const field = page.locator(selector);
  if ((await field.count()) > 0) {
    await field.fill(value).catch(() => {});
  }
}
