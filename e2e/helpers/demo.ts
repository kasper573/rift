import type { Browser, BrowserContext, Page, TestInfo } from "@playwright/test";
import { basename, extname, join } from "node:path";

import { VIDEO } from "../demo.config";
import { register } from "./account";
import { waitForWorld } from "./game";
import { loadReference } from "./image";
import { startRecording } from "./recording";
import { tapSoundtrack } from "./soundtrack";

export interface Chapter {
  summary: string;
  // Runs before recording starts. Defaults to a fresh player standing in the world.
  setup?: (page: Page, cast: Cast) => Promise<void>;
  play: (page: Page, cast: Cast) => Promise<void>;
}

export interface Cast {
  // Another player's browser, off camera.
  newPage(): Promise<Page>;
}

interface Fixtures {
  page: Page;
  browser: Browser;
  baseURL: string | undefined;
}

const TITLE_MS = 3000;
const OUTRO_MS = 1500;

// The body of one showcase chapter, `test("<title>", chapter({ ... }))`: it records to
// `<chapter file name>.webm`, opening on a card with the test's title and the summary.
export function chapter({ summary, setup = enterWorld, play }: Chapter) {
  return async ({ page, browser, baseURL }: Fixtures, info: TestInfo): Promise<void> => {
    const extras: BrowserContext[] = [];
    const cast: Cast = {
      newPage: async () => {
        const context = await browser.newContext({ baseURL, viewport: VIDEO, ignoreHTTPSErrors: true });
        extras.push(context);
        return context.newPage();
      },
    };
    try {
      await tapSoundtrack(page);
      await setup(page, cast);
      const recording = await startRecording(page, VIDEO, info.outputPath());
      try {
        await page.screencast.showActions({ cursor: "pointer", position: "bottom-right", fontSize: 18, duration: 250 });
        await page.screencast.showOverlay(titleCard(info.title, summary), { duration: TITLE_MS });
        await play(page, cast);
        await page.waitForTimeout(OUTRO_MS);
      } finally {
        // Kept on failure too: the recording is the quickest way to see what went wrong.
        await recording.finish(videoPath(info.file));
      }
    } finally {
      // The browser outlives the chapter, and so would these players, walking into the next one.
      await Promise.all(extras.map((context) => context.close()));
    }
  };
}

export async function enterWorld(page: Page): Promise<void> {
  await register(page);
  await waitForWorld(page, loadReference("island.png"));
}

const hideCaption = new WeakMap<Page, () => Promise<void>>();

export interface CaptionOptions {
  durationMs?: number;
  // "top" keeps the caption clear of whatever the game draws along the bottom edge.
  at?: "top" | "bottom";
}

// Stays up while the chapter plays on, until the next caption replaces it or its time is up. (An
// overlay shown with a duration would hold the chapter still for all of it.)
export async function caption(
  page: Page,
  text: string,
  { durationMs = 3500, at = "bottom" }: CaptionOptions = {},
): Promise<void> {
  await hideCaption.get(page)?.();
  const overlay = await page.screencast.showOverlay(subtitle(text, at));
  const hide = () => overlay.dispose().catch(() => {});
  const timer = setTimeout(hide, durationMs);
  hideCaption.set(page, () => {
    clearTimeout(timer);
    return hide();
  });
}

const INSET_REFRESH_MS = 200;

// The other browser is off camera, so its screen reaches the video only as this live picture.
export async function inset(page: Page, other: Page, label: string): Promise<() => Promise<void>> {
  let running = true;
  let shown: { dispose(): Promise<void> } | undefined;
  const refreshing = (async () => {
    while (running) {
      const shot = await other.screenshot({ type: "jpeg", quality: 70 }).catch(() => undefined);
      if (shot && running) {
        const next = await page.screencast.showOverlay(insetFrame(shot.toString("base64"), label));
        await shown?.dispose().catch(() => {});
        shown = next;
      }
      await new Promise((resolve) => setTimeout(resolve, INSET_REFRESH_MS));
    }
  })();
  return async () => {
    running = false;
    await refreshing;
    await shown?.dispose().catch(() => {});
  };
}

function insetFrame(jpeg: string, label: string): string {
  return `<div style="position:fixed;left:16px;top:168px;width:416px;border-radius:10px;overflow:hidden;
      border:3px solid rgba(255,255,255,.85);box-shadow:0 6px 24px rgba(0,0,0,.55);background:#000">
    <img src="data:image/jpeg;base64,${jpeg}" style="display:block;width:100%">
    <div style="position:absolute;left:0;top:0;padding:4px 10px;border-bottom-right-radius:8px;
      background:rgba(0,0,0,.72);color:#fff;font:600 16px/1.3 system-ui,sans-serif">${escape(label)}</div></div>`;
}

function titleCard(title: string, summary: string): string {
  return `<div style="position:fixed;inset:0;display:flex;flex-direction:column;align-items:center;
      justify-content:center;gap:14px;background:rgba(8,10,16,.6);backdrop-filter:blur(10px);
      font-family:system-ui,sans-serif;color:#fff">
    <div style="font:700 64px/1.1 system-ui,sans-serif;letter-spacing:.5px">${escape(title)}</div>
    <div style="font:400 26px/1.3 system-ui,sans-serif;color:#cbd5e1">${escape(summary)}</div></div>`;
}

function subtitle(text: string, at: "top" | "bottom"): string {
  return `<div style="position:fixed;left:0;right:0;${at}:48px;display:flex;justify-content:center">
    <div style="max-width:70%;padding:8px 18px;border-radius:8px;background:rgba(0,0,0,.72);color:#fff;
      font:500 22px/1.35 system-ui,sans-serif;text-align:center">${escape(text)}</div></div>`;
}

function escape(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function videoPath(chapterFile: string): string {
  const dir = process.env.RIFT_DEMO_DIR;
  if (!dir) throw new Error("RIFT_DEMO_DIR is not set");
  return join(dir, `${basename(chapterFile, extname(chapterFile))}.webm`);
}
