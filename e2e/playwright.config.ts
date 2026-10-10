import { defineConfig } from "@playwright/test";

const domain = process.env.RIFT_DOMAIN ?? "rift.lan";
export const baseURL = process.env.RIFT_E2E_URL ?? `https://${domain}`;

// rift.lan only resolves on the LAN; map it to loopback in-browser (a Chromium-only flag — headed
// Firefox/WebKit get the same from /etc/hosts and Firefox's prefs below). Skipped for a real URL.
export const hostResolver = process.env.RIFT_E2E_URL
  ? []
  : [`--host-resolver-rules=MAP ${domain} 127.0.0.1,MAP *.${domain} 127.0.0.1`];

// Browsers run on machines people are using, so the game's sound must never reach a real audio device:
// each engine gets a fake one instead.
export const chromiumSilence = ["--disable-audio-output"];
const firefoxSilence = { "media.cubeb.force_null_context": true };
const webkitSilence = { ...process.env, GST_PLUGIN_FEATURE_RANK: "fakeaudiosink:MAX" };

// Chrome's own software renderer (SwiftShader) runs the game several times slower than Mesa's, too
// slow for real input to keep up with, so Chrome renders through the display's GL like the others.
// Chrome blocklists software GL, which is all a runner without a GPU has.
const chromiumArgs = [
  "--use-gl=angle",
  "--use-angle=gl",
  "--ignore-gpu-blocklist",
  "--no-sandbox",
  "--disable-dev-shm-usage",
  ...chromiumSilence,
  ...hostResolver,
];

// Headed, on a virtual display: Firefox/WebKit can't do WebGL headless on Linux, and Chrome's GL
// needs a display to render through.
const browsers = {
  chrome: { browserName: "chromium", channel: "chrome", headless: false, launchOptions: { args: chromiumArgs } },
  // Edge is the same engine as Chrome (Chromium) — off for now to save a CI project. Re-add for
  // Edge-channel-specific coverage.
  // edge: { browserName: "chromium", channel: "msedge", headless: false, launchOptions: { args: chromiumArgs } },
  firefox: {
    browserName: "firefox",
    headless: false,
    launchOptions: {
      firefoxUserPrefs: {
        // Allow the software renderer past the GPU blocklist.
        "webgl.force-enabled": true,
        "webgl.disabled": false,
        "webgl.disable-fail-if-major-performance-caveat": true,
        // Firefox takes no host-resolver flag; map the stack's domains to loopback here instead.
        "network.dns.localDomains": process.env.RIFT_E2E_URL
          ? ""
          : `${domain},auth.${domain},game-server.${domain}`,
        ...firefoxSilence,
      },
    },
  },
  safari: { browserName: "webkit", headless: false, launchOptions: { env: webkitSilence } },
} as const;

// The suite must pass at any screen a player might use. The smallest one the game supports is the
// cheapest for software WebGL to render, by several times.
const resolutions = {
  smallest: { width: 640, height: 400 },
  // Mobile portrait is off for now (halves the matrix). Re-add for mobile-viewport coverage.
  // portrait: { width: 480, height: 844 },
} as const;

type BrowserName = keyof typeof browsers;
type Resolution = keyof typeof resolutions;

// Locally: one fast combination. CI sets E2E_ALL_BROWSERS to fan out across every browser × resolution.
const matrix: Array<[BrowserName, Resolution]> = process.env.E2E_ALL_BROWSERS
  ? (Object.keys(browsers) as BrowserName[]).flatMap((browser) =>
      (Object.keys(resolutions) as Resolution[]).map((res) => [browser, res] as [BrowserName, Resolution]),
    )
  : [["chrome", "smallest"]];

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  // Half the cores, so the suite scales with the runner — the slow software-WebGL browsers need real
  // cores per worker. LP_NUM_THREADS in `just e2e-run` keeps total render threads near the core count.
  workers: process.env.E2E_WORKERS ?? "50%",
  // Unoptimized software WebGL is slow to bring up a canvas and spawn the player.
  timeout: 240_000,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : [["list"]],
  use: {
    baseURL,
    // The local stack uses Caddy's internal CA; the suite isn't testing TLS.
    ignoreHTTPSErrors: true,
    trace: "retain-on-failure",
    video: "retain-on-failure",
    screenshot: "only-on-failure",
    // Generous: a CPU-starved headed browser under parallel load loads even plain auth pages slowly.
    actionTimeout: 60_000,
    navigationTimeout: 90_000,
  },
  projects: matrix.map(([browser, resolution]) => ({
    name: `${browser}-${resolution}`,
    use: { ...browsers[browser], viewport: resolutions[resolution], deviceScaleFactor: 1 },
  })),
});
