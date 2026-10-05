import { expect, type Page } from "@playwright/test";

import { decode, resemblance, type Image } from "./image";

// "Basically the same place" as a reference: the matching map scores ~0.83, the other ~0.03, so 0.5
// is a wide margin. waitForWorld waits for the spawn map to cross it; the tests assert against it.
export const MAP_MATCH = 0.5;
// Unoptimized software WebGL is slow to bring up a canvas and render the first frames.
const WORLD_TIMEOUT = 120_000;

export type Tile = [number, number];

export interface TileRect {
  origin: Tile;
  size: [number, number];
}

export interface Body {
  id: string;
  name: string;
  model: string;
  flies: boolean;
  player: boolean;
  at: Tile;
  aim: Tile;
  hitbox: TileRect;
  health: number;
  max_health: number;
  npc: string | null;
  role: string | null;
  friendly: boolean;
  locked: boolean;
  marks: string[];
  badges: string[];
}

export interface Fixture {
  id: string;
  prop: string;
  at: Tile;
  aim: Tile;
  hitbox: TileRect;
  marks: string[];
}

export interface GroundItem {
  item: string;
  count: number;
  at: Tile;
}

export interface Exit {
  name: string;
  to: string;
  at: Tile;
}

export interface UiElement {
  text: string | null;
  image: string | null;
  editable: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Cover {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ShopOffer {
  item: string;
  count: number;
  left: number | null;
  refusal: string | null;
}

export interface Stack {
  item: string;
  count: number;
}

export interface Shop {
  shop: string;
  offers: ShopOffer[];
  buyback: Stack[];
}

export interface StageChoice {
  label: string;
  locked: boolean;
  rect: Cover | null;
}

export interface QuestEntry {
  quest: string;
  ready: boolean;
  left: number | null;
  progress: [number, number][];
}

export interface Quests {
  active: QuestEntry[];
  finished: { quest: string; result: "Completed" | "Failed" }[];
  tracked: string[];
}

export interface Stage {
  node: string;
  with: string | null;
  line: number;
  lines: number;
  speaker: string | null;
  typing: boolean;
  choices: StageChoice[];
  waiting: string | null;
}

export interface LaneLine {
  by: string;
  text: string;
}

export interface Lane {
  showing: LaneLine | null;
  next: LaneLine | null;
  missed: LaneLine[];
}

// What the client sees this frame. Positions are world tiles, which `view` maps onto canvas pixels
// (origin + tile × tile_size); `ui` rects are canvas pixels already. `viewpoint` is the character the
// camera follows: `me`, or the player a spectator watches. `walkable` holds the tiles on screen that you
// can walk to from where you stand.
export interface Snapshot {
  view: { origin: Tile; tile_size: Tile } | null;
  area: string | null;
  me: Body | null;
  viewpoint: Body | null;
  actors: Body[];
  props: Fixture[];
  items: GroundItem[];
  portals: Exit[];
  markers: { name: string; at: Tile }[];
  walkable: Tile[];
  area_size: [number, number] | null;
  safe_zones: TileRect[];
  ui: UiElement[];
  covered: Cover[];
  stage: Stage | null;
  announcement: Lane;
  history: boolean;
  shop: Shop | null;
  bag: Stack[];
  quests: Quests;
}

// Waits until the world is on screen — polls until the captured frame resembles the spawn map.
export async function waitForWorld(page: Page, spawnMap: Image): Promise<void> {
  await canvas(page).waitFor({ state: "visible", timeout: WORLD_TIMEOUT });
  await expect
    .poll(async () => resemblance(await captureScene(page), spawnMap), {
      message: "the world never rendered (the canvas stayed blank)",
      timeout: WORLD_TIMEOUT,
      intervals: [500],
    })
    .toBeGreaterThanOrEqual(MAP_MATCH);
}

export async function captureScene(page: Page): Promise<Image> {
  return decode(await canvas(page).screenshot());
}

export async function probe(page: Page): Promise<Snapshot> {
  let snapshot: Snapshot | null = null;
  await expect
    .poll(
      async () => {
        snapshot = await readProbe(page);
        return snapshot !== null;
      },
      { message: "the client never published a probe snapshot", intervals: [50] },
    )
    .toBe(true);
  return snapshot!;
}

export async function waitFor<T>(
  page: Page,
  find: (snapshot: Snapshot) => T | null | undefined | false,
  message: string,
  timeout = 30_000,
): Promise<T> {
  let found: T | null | undefined | false = undefined;
  await expect
    .poll(
      async () => {
        found = find(await probe(page));
        return Boolean(found);
      },
      { message, timeout, intervals: [100] },
    )
    .toBe(true);
  return found as T;
}

export async function hoverTile(page: Page, tile: Tile): Promise<void> {
  const { x, y } = await tilePoint(page, tile);
  await page.mouse.move(x, y, { steps: 12 });
}

// A real mouse click, so it takes the same path through the game's input gestures as a player's.
export async function clickTile(page: Page, tile: Tile): Promise<void> {
  await hoverTile(page, tile);
  await click(page);
}

// Waits for the player to come to rest rather than to arrive: anyone standing on the tile stops them
// short of it.
export async function walkTo(page: Page, tile: Tile): Promise<Tile> {
  await clickTile(page, tile);
  return waitUntilStill(page);
}

export async function waitUntilStill(page: Page, timeout = 30_000): Promise<Tile> {
  const deadline = Date.now() + timeout;
  let last: Tile | undefined;
  let still = 0;
  await page.waitForTimeout(400);
  while (still < 3) {
    if (Date.now() > deadline) throw new Error("the player never came to rest");
    const at = (await probe(page)).me?.at;
    still = at && last && distance(at, last) === 0 ? still + 1 : 0;
    last = at;
    await page.waitForTimeout(250);
  }
  return last!;
}

// Picking up competes with whatever else is on that tile (a monster standing over the loot takes the
// click as an attack), so keep clicking until the item is gone.
export async function pickUp(page: Page, item: GroundItem, timeout = 30_000): Promise<void> {
  const deadline = Date.now() + timeout;
  const lying = ({ items }: Snapshot) =>
    items.some((other) => other.item === item.item && distance(other.at, item.at) < 0.1);
  while (lying(await probe(page))) {
    if (Date.now() > deadline) throw new Error(`never picked up ${item.item}`);
    await clickTile(page, item.at);
    await page.waitForTimeout(2500);
  }
}

// A click on someone or something interacts with it instead of walking. The margin covers a body
// that steps under the cursor between the probe and the click.
export function occupied(snapshot: Snapshot, tile: Tile): boolean {
  const margin = 0.5;
  return [...snapshot.actors, ...snapshot.props].some(({ hitbox: { origin, size } }) =>
    tile[0] > origin[0] - margin &&
    tile[1] > origin[1] - margin &&
    tile[0] < origin[0] + size[0] + margin &&
    tile[1] < origin[1] + size[1] + margin,
  );
}

export function warps(snapshot: Snapshot, tile: Tile): boolean {
  return snapshot.portals.some((portal) => distance(tile, portal.at) < 2);
}

// Safety goes by whole tiles: a spot is safe when the center of its tile lies in a safe zone.
export function safe(snapshot: Snapshot, at: Tile): boolean {
  const center: Tile = [Math.round(at[0]), Math.round(at[1])];
  return snapshot.safe_zones.some((zone) => within(zone, center));
}

// A few steps past the nearest edge of the safe zone you stand in, on the map.
export function wayOut(snapshot: Snapshot, depth = 3): Tile | undefined {
  const me = snapshot.me?.at;
  const zone = me && snapshot.safe_zones.find((zone) => within(zone, me));
  if (!me || !zone || !snapshot.area_size) return undefined;
  const [width, height] = snapshot.area_size;
  const [left, top] = zone.origin;
  const [right, bottom] = [left + zone.size[0], top + zone.size[1]];
  const exits: Tile[] = [
    [left - depth, me[1]],
    [right + depth, me[1]],
    [me[0], top - depth],
    [me[0], bottom + depth],
  ];
  return closestTile(
    exits.filter(([x, y]) => x >= 0 && y >= 0 && x < width && y < height),
    me,
  );
}

// Only on-screen tiles can be clicked, so a far target is approached hop by hop. A function target is
// re-read every hop, for things that move. The last click waits for the camera to settle, or it lands
// where the target was a moment ago.
export async function travelTo(
  page: Page,
  target: Tile | ((snapshot: Snapshot) => Tile | undefined),
  timeout = 60_000,
): Promise<void> {
  const locate = typeof target === "function" ? target : () => target;
  const deadline = Date.now() + timeout;
  for (;;) {
    const snapshot = await probe(page);
    if (snapshot.stage) {
      await leaveConversation(page);
      continue;
    }
    const tile = locate(snapshot);
    if (!tile) throw new Error("the travel target is gone");
    const size = await canvasSize(page);
    if (clickable(snapshot, tile, size)) {
      await waitUntilStill(page);
      const settled = locate(await probe(page));
      if (!settled) throw new Error("the travel target is gone");
      await clickTile(page, settled);
      return;
    }
    if (Date.now() > deadline) throw new Error(`never got ${tile} on screen`);
    const moves = (step: Tile) => !snapshot.me || distance(step, snapshot.me.at) >= 2;
    const hop = closestTile(
      snapshot.walkable.filter(
        (step) =>
          moves(step) && !occupied(snapshot, step) && !warps(snapshot, step) && clickable(snapshot, step, size),
      ),
      tile,
    );
    if (!hop) throw new Error("no walkable tile on screen");
    await clickTile(page, hop);
    await page.waitForTimeout(1200);
  }
}

// Text and icon paths match exactly as strings, or by pattern.
export type UiMatch = string | RegExp | ((element: UiElement) => boolean);

export function findUi(snapshot: Snapshot, match: UiMatch): UiElement | undefined {
  const test = (value: string | null) =>
    value !== null && (typeof match === "string" ? value === match : (match as RegExp).test(value));
  return snapshot.ui.find((element) =>
    typeof match === "function" ? match(element) : test(element.text) || test(element.image),
  );
}

export async function hoverUi(page: Page, match: UiMatch): Promise<void> {
  const { x, y } = await uiPoint(page, match);
  await page.mouse.move(x, y, { steps: 12 });
}

export async function clickUi(page: Page, match: UiMatch): Promise<void> {
  const { x, y } = await uiPoint(page, match);
  await page.mouse.move(x, y, { steps: 12 });
  await click(page);
}

export async function rightClickUi(page: Page, match: UiMatch): Promise<void> {
  const { x, y } = await uiPoint(page, match);
  await page.mouse.move(x, y, { steps: 12 });
  await page.mouse.down({ button: "right" });
  await page.mouse.up({ button: "right" });
}

export async function dragUiOnto(page: Page, from: UiMatch, onto: UiMatch): Promise<void> {
  const start = await uiPoint(page, from);
  const end = await uiPoint(page, onto);
  await page.mouse.move(start.x, start.y, { steps: 12 });
  await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 30 });
  await page.mouse.up();
}

export function holding(snapshot: Snapshot, item: string): number {
  return snapshot.bag.filter((stack) => stack.item === item).reduce((total, stack) => total + stack.count, 0);
}

export async function dragUi(page: Page, match: UiMatch, by: { x: number; y: number }): Promise<void> {
  const from = await uiPoint(page, match);
  await page.mouse.move(from.x, from.y, { steps: 12 });
  await page.mouse.down();
  await page.mouse.move(from.x + by.x, from.y + by.y, { steps: 30 });
  await page.mouse.up();
}

// Keyboard input goes to whatever has focus; the game only hears it while its canvas does.
export async function focusGame(page: Page): Promise<void> {
  await canvas(page).focus();
}

export function nearest<T extends { at: Tile }>(things: T[], from: Tile): T | undefined {
  return things
    .slice()
    .sort((a, b) => distance(a.at, from) - distance(b.at, from))
    .at(0);
}

export function distance(a: Tile, b: Tile): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1]);
}

async function readProbe(page: Page): Promise<Snapshot | null> {
  const json = await page.evaluate(() => {
    const hook = (window as unknown as { rift_probe?: () => string | undefined }).rift_probe;
    return (typeof hook === "function" && hook()) || null;
  });
  return json === null ? null : (JSON.parse(json) as Snapshot);
}

async function uiPoint(page: Page, match: UiMatch): Promise<{ x: number; y: number }> {
  const element = await waitFor(page, (snapshot) => findUi(snapshot, match), `no UI element matches ${match}`);
  const box = await canvasBox(page);
  return { x: box.x + element.x + element.width / 2, y: box.y + element.y + element.height / 2 };
}

async function tilePoint(page: Page, tile: Tile): Promise<{ x: number; y: number }> {
  const { view } = await probe(page);
  if (!view) throw new Error("the world camera isn't up yet");
  const box = await canvasBox(page);
  return {
    x: box.x + view.origin[0] + tile[0] * view.tile_size[0],
    y: box.y + view.origin[1] + tile[1] * view.tile_size[1],
  };
}

// A tile's width clear of the edges, where the HUD sits.
function clickable(snapshot: Snapshot, tile: Tile, size: { width: number; height: number }): boolean {
  if (!snapshot.view) return false;
  const margin = 1;
  const x = snapshot.view.origin[0] + tile[0] * snapshot.view.tile_size[0];
  const y = snapshot.view.origin[1] + tile[1] * snapshot.view.tile_size[1];
  const [mx, my] = snapshot.view.tile_size.map((side) => Math.abs(side) * margin);
  const onScreen = x >= mx && y >= my && x <= size.width - mx && y <= size.height - my;
  const under = (cover: Cover) =>
    x >= cover.x - mx && y >= cover.y - my && x <= cover.x + cover.width + mx && y <= cover.y + cover.height + my;
  return onScreen && !snapshot.covered.some(under);
}

// A walk click can land on a townsperson who strolled under it, and talking locks every later click.
export async function leaveConversation(page: Page): Promise<void> {
  await focusGame(page);
  await page.keyboard.press("Escape");
  await waitFor(page, ({ stage }) => stage === null, "the stray conversation never closed");
}

export function closestTile(tiles: Tile[], target: Tile): Tile | undefined {
  return tiles
    .slice()
    .sort((a, b) => distance(a, target) - distance(b, target))
    .at(0);
}

async function click(page: Page): Promise<void> {
  await page.mouse.down();
  await page.mouse.up();
}

async function canvasBox(page: Page): Promise<{ x: number; y: number; width: number; height: number }> {
  const box = await canvas(page).boundingBox();
  if (!box) throw new Error("the game canvas isn't on the page");
  return box;
}

async function canvasSize(page: Page): Promise<{ width: number; height: number }> {
  const { width, height } = await canvasBox(page);
  return { width, height };
}

// The canvas has no semantic role; target it by id.
function canvas(page: Page) {
  return page.locator("#glcanvas");
}

export function onQuest(snapshot: Snapshot, quest: string): QuestEntry | undefined {
  return snapshot.quests.active.find((active) => active.quest === quest);
}

export function finished(snapshot: Snapshot, quest: string): "Completed" | "Failed" | undefined {
  return snapshot.quests.finished.find((done) => done.quest === quest)?.result;
}

function within({ origin, size }: TileRect, at: Tile): boolean {
  return at[0] >= origin[0] && at[1] >= origin[1] && at[0] < origin[0] + size[0] && at[1] < origin[1] + size[1];
}
