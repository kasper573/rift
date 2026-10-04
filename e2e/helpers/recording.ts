import type { Page } from "@playwright/test";
import { execFileSync, spawn } from "node:child_process";
import { once } from "node:events";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Writable } from "node:stream";

import { collectSoundtrack, type Soundtrack } from "./soundtrack";

const FPS = 30;

export interface Recording {
  finish(out: string): Promise<void>;
}

// Playwright's built-in recorder shifts frames off their capture time, too far to lay a soundtrack
// against. So every frame lands here at the moment the browser presented it, on the same clock the
// soundtrack is stamped with.
export async function startRecording(
  page: Page,
  size: { width: number; height: number },
  scratchDir: string,
): Promise<Recording> {
  const silent = join(scratchDir, "video.webm");
  const encoder = spawn(
    "ffmpeg",
    [
      ...["-v", "error", "-y", "-f", "image2pipe", "-framerate", `${FPS}`, "-c:v", "mjpeg", "-i", "pipe:0"],
      ...["-c:v", "libvpx", "-deadline", "realtime", "-cpu-used", "8", "-b:v", "4M", silent],
    ],
    { stdio: ["pipe", "ignore", "inherit"] },
  );
  const exited = once(encoder, "exit");
  let startedAt: number | undefined;
  let shown: Buffer | undefined;
  let frames = 0;
  let written = Promise.resolve();
  const showUntil = (time: number) => {
    const frame = shown;
    if (startedAt === undefined || !frame) return;
    for (const due = Math.round(((time - startedAt) * FPS) / 1000); frames < due; frames++) {
      written = written.then(() => write(encoder.stdin, frame));
    }
  };

  await page.screencast.start({
    size,
    onFrame: ({ data, timestamp }) => {
      startedAt ??= timestamp;
      showUntil(timestamp);
      shown = data;
    },
  });

  return {
    async finish(out) {
      await page.screencast.stop();
      showUntil(Date.now());
      await written;
      encoder.stdin.end();
      const [code] = await exited;
      if (code !== 0) throw new Error(`encoding the video failed: ffmpeg exited with ${code}`);
      if (startedAt === undefined) throw new Error("the page never presented a frame");
      mux(silent, frames / FPS, startedAt, await collectSoundtrack(page), join(scratchDir, "soundtrack.webm"), out);
    },
  };
}

// The audio stream runs the video's exact length (silence when the page played none), so recordings
// join without re-encoding and without drifting out of sync.
function mux(
  video: string,
  seconds: number,
  videoStartedAt: number,
  soundtrack: Soundtrack | null,
  scratch: string,
  out: string,
): void {
  if (soundtrack) writeFileSync(scratch, soundtrack.webm);
  const audio = soundtrack ? ["-i", scratch] : ["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo"];
  const offsetMs = soundtrack ? soundtrack.startedAt - videoStartedAt : 0;
  const align =
    offsetMs >= 0 ? `adelay=${offsetMs}:all=1` : `atrim=start=${-offsetMs / 1000},asetpts=PTS-STARTPTS`;
  execFileSync("ffmpeg", [
    ...["-v", "error", "-y", "-i", video, ...audio],
    ...["-filter_complex", `[1:a]${align},apad=whole_dur=${seconds},atrim=end=${seconds}[a]`],
    ...["-map", "0:v", "-map", "[a]", "-t", `${seconds}`],
    ...["-c:v", "copy", "-c:a", "libopus", "-b:a", "128k", "-ar", "48000", "-ac", "2", out],
  ]);
}

function write(stream: Writable, chunk: Buffer): Promise<void> {
  return new Promise((resolve, reject) => stream.write(chunk, (error) => (error ? reject(error) : resolve())));
}
