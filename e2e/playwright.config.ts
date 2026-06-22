import { defineConfig } from "@playwright/test";

// The stack's domain is configured in one place (docker/.env.test's RIFT_DOMAIN); the suite derives
// its URL from it. A RIFT_E2E_URL override points the suite at a real deployment instead.
const domain = process.env.RIFT_DOMAIN ?? "rift.lan";
const baseURL = process.env.RIFT_E2E_URL ?? `https://${domain}`;

// rift.lan and its subdomains only resolve on the LAN, so map them to loopback inside the browser —
// the suite reaches the published ports without depending on DNS. This flag is Chromium-only; the
// headed Firefox/WebKit projects get the same mapping from /etc/hosts in CI. A real deployment
// resolves normally, so skip it when RIFT_E2E_URL points the suite elsewhere.
const hostResolver = process.env.RIFT_E2E_URL
  ? []
  : [`--host-resolver-rules=MAP ${domain} 127.0.0.1,MAP *.${domain} 127.0.0.1`];

// Chrome 137+ no longer falls back to SwiftShader on its own, so force software WebGL2 explicitly —
// the game renders with no GPU (CI runners have none). The sandbox/shm flags are the usual CI ones.
const chromiumArgs = [
  "--use-gl=angle",
  "--use-angle=swiftshader",
  "--enable-unsafe-swiftshader",
  "--no-sandbox",
  "--disable-dev-shm-usage",
  ...hostResolver,
];

// Firefox and WebKit can't drive WebGL headless on Linux, so they run headed against a virtual
// display (xvfb-run, see e2e/README.md). Chrome and Edge render headless via SwiftShader. Local runs
// only touch Chrome, so they need neither xvfb nor /etc/hosts.
const browsers = {
  chrome: {
    browserName: "chromium",
    channel: "chrome",
    launchOptions: { args: chromiumArgs },
  },
  edge: {
    browserName: "chromium",
    channel: "msedge",
    launchOptions: { args: chromiumArgs },
  },
  firefox: {
    browserName: "firefox",
    headless: false,
    launchOptions: {
      firefoxUserPrefs: {
        // Bypass the GPU blocklist and allow the software renderer so WebGL works under xvfb.
        "webgl.force-enabled": true,
        "webgl.disabled": false,
        "webgl.disable-fail-if-major-performance-caveat": true,
        // Firefox can't take a host-resolver flag, so resolve the stack's domains to loopback here
        // (the chromium equivalent of --host-resolver-rules).
        "network.dns.localDomains": process.env.RIFT_E2E_URL
          ? ""
          : `${domain},auth.${domain},game-server.${domain}`,
      },
    },
  },
  safari: { browserName: "webkit", headless: false },
} as const;

const resolutions = {
  desktop: { width: 1280, height: 800 },
  landscape: { width: 844, height: 480 },
  portrait: { width: 480, height: 844 },
} as const;

type BrowserName = keyof typeof browsers;
type Resolution = keyof typeof resolutions;

// Locally we test one combination — chrome at desktop size — for a fast, deterministic loop. CI sets
// E2E_ALL_BROWSERS to fan out across every browser × resolution.
const matrix: Array<[BrowserName, Resolution]> = process.env.E2E_ALL_BROWSERS
  ? (Object.keys(browsers) as BrowserName[]).flatMap((browser) =>
      (Object.keys(resolutions) as Resolution[]).map((res) => [browser, res] as [BrowserName, Resolution]),
    )
  : [["chrome", "desktop"]];

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  // One worker in CI: software WebGL is CPU-bound, and two heavy canvases sharing a runner drop the
  // client's frame rate enough that held clicks miss. Local runs (one browser) can parallelize.
  workers: process.env.CI ? Number(process.env.E2E_WORKERS ?? 1) : 2,
  // Unoptimized software WebGL is slow: the canvas can take tens of seconds to come up and the player
  // tens more to spawn, so a registration + spawn + gameplay action needs generous room.
  timeout: 240_000,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : [["list"]],
  use: {
    baseURL,
    // Caddy serves the local stack from its own internal CA; the suite isn't checking TLS.
    ignoreHTTPSErrors: true,
    trace: "retain-on-failure",
    video: "retain-on-failure",
    screenshot: "only-on-failure",
    actionTimeout: 30_000,
    navigationTimeout: 60_000,
  },
  projects: matrix.map(([browser, resolution]) => ({
    name: `${browser}-${resolution}`,
    use: { ...browsers[browser], viewport: resolutions[resolution], deviceScaleFactor: 1 },
  })),
});
