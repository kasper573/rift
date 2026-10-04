import type { Page } from "@playwright/test";

export interface Soundtrack {
  startedAt: number;
  webm: Buffer;
}

// The browser plays into a fake audio device (nothing reaches the machine's speakers), so the
// soundtrack is recorded inside the page instead: everything the game sends to its audio output is
// also sent to a recorder. Install before the game loads.
export async function tapSoundtrack(page: Page): Promise<void> {
  await page.addInitScript(installTap);
}

export async function collectSoundtrack(page: Page): Promise<Soundtrack | null> {
  const recorded = await page
    .evaluate(() => (window as unknown as { rift_soundtrack?: () => Promise<RecordedAudio | null> }).rift_soundtrack?.())
    .catch(() => null);
  return recorded ? { startedAt: recorded.startedAt, webm: Buffer.from(recorded.base64, "base64") } : null;
}

interface RecordedAudio {
  startedAt: number;
  base64: string;
}

// Runs in the page, so it must be self-contained.
function installTap(): void {
  const connect = AudioNode.prototype.connect as (this: AudioNode, ...args: unknown[]) => unknown;
  const taps = new Map<BaseAudioContext, MediaStreamAudioDestinationNode>();
  let recording: { startedAt: number; recorder: MediaRecorder; chunks: Blob[] } | undefined;

  const record = (tap: MediaStreamAudioDestinationNode) => {
    const recorder = new MediaRecorder(tap.stream, { mimeType: "audio/webm;codecs=opus" });
    const chunks: Blob[] = [];
    recorder.ondataavailable = (event) => chunks.push(event.data);
    recorder.start();
    recording = { startedAt: Date.now(), recorder, chunks };
  };

  // A stream with no samples coming in pauses, and the recording closes the gap and drifts out of
  // sync. So it's fed a muted signal to never fall quiet, and recorded only once its context runs.
  const tapOf = (context: BaseAudioContext) => {
    let tap = taps.get(context);
    if (!tap) {
      tap = (context as AudioContext).createMediaStreamDestination();
      taps.set(context, tap);
      const hold = context.createConstantSource();
      const mute = context.createGain();
      mute.gain.value = 0;
      connect.call(hold, mute);
      connect.call(mute, tap);
      hold.start();
      const created = tap;
      const start = () => {
        if (!recording && context.state === "running") record(created);
      };
      context.addEventListener("statechange", start);
      start();
    }
    return tap;
  };

  AudioNode.prototype.connect = function (this: AudioNode, ...args: unknown[]) {
    const connected = connect.apply(this, args);
    const [destination, output] = args;
    if (destination instanceof AudioDestinationNode && destination.context instanceof AudioContext) {
      try {
        connect.call(this, tapOf(destination.context), output ?? 0);
      } catch {
        // The game's own connection above stands; only the recording misses this node.
      }
    }
    return connected;
  } as typeof AudioNode.prototype.connect;

  (window as unknown as { rift_soundtrack: () => Promise<RecordedAudio | null> }).rift_soundtrack = async () => {
    if (!recording) return null;
    const { recorder, chunks, startedAt } = recording;
    if (recorder.state !== "inactive") {
      await new Promise((resolve) => {
        recorder.onstop = resolve;
        recorder.stop();
      });
    }
    const blob = new Blob(chunks, { type: recorder.mimeType });
    const base64 = await new Promise<string>((resolve) => {
      const reader = new FileReader();
      reader.onload = () => resolve((reader.result as string).split(",")[1]);
      reader.readAsDataURL(blob);
    });
    return { startedAt, base64 };
  };
}
