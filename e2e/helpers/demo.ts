import type { Browser, BrowserContext, Page, TestInfo } from "@playwright/test";
import { basename, extname, join } from "node:path";

import { VIDEO } from "../demo.config";
import { register } from "./account";
import { probe, waitForWorld, type SoundVoice } from "./game";
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
        await page.screencast.showActions({ cursor: "none", position: "bottom-right", fontSize: 18, duration: 250 });
        await mirrorCursor(page);
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

// The recording captures the page, not the system cursor, so a copy of whatever cursor the page
// shows under the mouse follows it on camera.
async function mirrorCursor(page: Page): Promise<void> {
  await page.evaluate(
    ({ arrow, beam }) => {
      const image = document.createElement("img");
      image.style.cssText = "position:fixed;left:0;top:0;pointer-events:none;z-index:2147483647;display:none";
      document.documentElement.append(image);
      let at: { x: number; y: number } | undefined;
      document.addEventListener("mousemove", (event) => (at = { x: event.clientX, y: event.clientY }), true);
      const follow = () => {
        const under = at && document.elementFromPoint(at.x, at.y);
        if (at && under) {
          const cursor = getComputedStyle(under).cursor;
          const custom = /url\("?([^")]+)"?\)\s+(\d+)\s+(\d+)/.exec(cursor);
          const [src, x, y] = custom
            ? [custom[1], Number(custom[2]), Number(custom[3])]
            : cursor === "text"
              ? [beam, 8, 12]
              : [arrow, 1, 1];
          if (image.getAttribute("src") !== src) image.src = src;
          image.style.transform = `translate(${at.x - x}px, ${at.y - y}px)`;
          image.style.display = "block";
        }
        requestAnimationFrame(follow);
      };
      requestAnimationFrame(follow);
    },
    { arrow: svgCursor(ARROW), beam: svgCursor(BEAM) },
  );
}

const ARROW = `<svg xmlns="http://www.w3.org/2000/svg" width="20" height="28"><path d="M1 1v22l6-6 4 9 3-1-4-9h8z" fill="#fff" stroke="#000" stroke-width="1.5"/></svg>`;
const BEAM = `<svg xmlns="http://www.w3.org/2000/svg" width="16" height="24"><path d="M4 2h8M8 2v20M4 22h8" fill="none" stroke="#fff" stroke-width="4"/><path d="M4 2h8M8 2v20M4 22h8" fill="none" stroke="#000" stroke-width="2"/></svg>`;

function svgCursor(svg: string): string {
  return `data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}`;
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

const MIXER_REFRESH_MS = 200;
const METER_RANGE_DB = 48;

// A soundtrack can't show which channel a track plays on, or how loud, so this panel does.
export async function soundscapeMixer(page: Page): Promise<() => Promise<void>> {
  let running = true;
  let shown: { dispose(): Promise<void> } | undefined;
  const refreshing = (async () => {
    while (running) {
      const voices = await probe(page)
        .then((snapshot) => snapshot.soundscape.voices)
        .catch(() => undefined);
      if (voices && running) {
        const next = await page.screencast.showOverlay(mixerPanel(voices));
        await shown?.dispose().catch(() => {});
        shown = next;
      }
      await new Promise((resolve) => setTimeout(resolve, MIXER_REFRESH_MS));
    }
  })();
  return async () => {
    running = false;
    await refreshing;
    await shown?.dispose().catch(() => {});
  };
}

function mixerPanel(voices: SoundVoice[]): string {
  const rows = voices
    .slice()
    .sort((a, b) => a.channel - b.channel || a.src.localeCompare(b.src))
    .map(
      ({ channel, category, src, heard }) => `<div style="display:flex;align-items:center;gap:10px;margin:4px 0">
        <span style="width:22px;color:#94a3b8">${channel}</span>
        <span style="width:120px">${escape(basename(src, extname(src)))}</span>
        <span style="width:70px;color:#94a3b8">${category}</span>
        <span style="flex:1;height:8px;border-radius:4px;background:rgba(255,255,255,.15);overflow:hidden">
          <span style="display:block;height:100%;width:${meter(heard)}%;background:#facc15"></span></span></div>`,
    )
    .join("");
  return `<div style="position:fixed;right:72px;top:40px;width:380px;padding:10px 14px;border-radius:10px;
      background:rgba(0,0,0,.72);color:#fff;font:500 15px/1.3 system-ui,sans-serif">
    <div style="font-weight:700;margin-bottom:4px">Soundscape channels</div>
    ${rows || '<div style="color:#94a3b8">silence</div>'}</div>`;
}

// Loudness is heard in decibels, so the bar spans METER_RANGE_DB of them down from full scale.
function meter(amplitude: number): number {
  const decibels = 20 * Math.log10(Math.max(amplitude, 1e-6));
  return Math.round(Math.min(1, Math.max(0, 1 + decibels / METER_RANGE_DB)) * 100);
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
