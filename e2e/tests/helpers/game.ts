import { expect, type Page } from "@playwright/test";

import { center, decode, diffFraction, greenMarker, sceneFraction, type Image } from "./image";

// The game draws every tile at a fixed 48 CSS px on every screen (render.rs), and the health bar
// sits 5 world px (×3) below the player's position.
const TILE_PX = 48;
const BAR_DROP_PX = 15;

// Hold the button down for a single rendered frame: long enough that press and release land in
// different frames (the client samples the button once per frame), short enough that only the
// initial move fires and the client's move-repeat never does. That matters for the portal — a repeat
// re-aims as the camera scrolls and cancels the cross. Registration can still lose the frame race on
// a starved renderer, so callers re-click until the player actually moves (clickUntil).
const CLICK_FRAMES = 1;

// A frame difference above this means the player moved (the camera scrolled); below it the frame is
// just idle water shimmer. Used to tell "the click hasn't taken yet, click again" from "it took,
// stop clicking and let it play out".
const MOVED = 0.08;

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

// Waits until the world is on screen and the local player has spawned and is controllable — its
// health bar is visible. On a slow renderer the player can take tens of seconds to appear after the
// tiles do; waiting for it here (rather than letting a gameplay action burn its budget waiting)
// keeps the action timeouts about the action itself.
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

// Clicks a tile offset from the player and waits for `done` to hold, re-clicking each poll only while
// the player still sits where it started (`before`). The single-frame click never triggers
// move-repeat, so once one registers the player acts on it cleanly; re-clicking after it has moved
// would re-aim and undo the action, so we stop then. This rides out the frame race on a slow renderer
// without ever canceling the move.
export async function clickUntil(
  page: Page,
  before: Image,
  tilesEast: number,
  tilesNorth: number,
  done: (scene: Image) => boolean,
  options: { message: string; timeout: number },
): Promise<void> {
  await expect
    .poll(
      async () => {
        const scene = await captureScene(page);
        if (done(scene)) return true;
        if (diffFraction(before, scene, 100) < MOVED) {
          await clickFromPlayer(page, tilesEast, tilesNorth);
        }
        return false;
      },
      { message: options.message, timeout: options.timeout, intervals: [700] },
    )
    .toBe(true);
}

// Presses and holds the button at a viewport point across CLICK_FRAMES rendered frames so the client
// samples it as a single click.
async function press(page: Page, x: number, y: number): Promise<void> {
  await page.mouse.move(x, y);
  await page.mouse.down();
  await waitForFrames(page, CLICK_FRAMES);
  await page.mouse.up();
}

// Resolves after the page has painted `frames` animation frames — i.e. the game's render loop has
// ticked that many times. Falls back to a wall-clock cap so a throttled tab can't hang the click.
async function waitForFrames(page: Page, frames: number): Promise<void> {
  await page.evaluate(
    (n) =>
      new Promise<void>((resolve) => {
        let seen = 0;
        const tick = () => (++seen >= n ? resolve() : requestAnimationFrame(tick));
        requestAnimationFrame(tick);
        setTimeout(resolve, 5000);
      }),
    frames,
  );
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
