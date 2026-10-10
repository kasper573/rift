import type { Page } from "@playwright/test";

export interface Account {
  username: string;
  password: string;
}

export type KeycloakGroup = "admin" | "spectator";

const REALM = "rift";

let counter = 0;

// Registers a fresh throwaway account by clicking through the sign-in flow, then leaves the redirect
// back to the game for waitForWorld. A unique username per call avoids parallel tests colliding.
export async function register(page: Page, { typingDelayMs }: { typingDelayMs?: number } = {}): Promise<Account> {
  if (new URL(page.url()).pathname !== "/play") await page.goto("/play");
  await page.getByRole("button", { name: "Sign in to play" }).click();
  await page.getByRole("link", { name: /register/i }).click();

  const account = newAccount();
  const type = (selector: string, value: string) =>
    typingDelayMs === undefined
      ? page.locator(selector).fill(value)
      : page.locator(selector).pressSequentially(value, { delay: typingDelayMs });
  // Field ids are Keycloak's stable, locale-independent contract.
  await type("#username", account.username);
  await type("#email", email(account));
  await type("#password", account.password);
  await type("#password-confirm", account.password);
  await fillIfPresent(page, "#firstName", account.username);
  await fillIfPresent(page, "#lastName", account.username);
  await page.getByRole("button", { name: /register/i }).click();
  return account;
}

// Ends back on the game's page: the login redirects through the site to it, and until it lands, anything
// that reads the page reads one on its way out.
export async function signIn(page: Page, account: Account): Promise<void> {
  await page.goto("/play");
  const play = page.url();
  await page.getByRole("button", { name: "Sign in to play" }).click();
  await page.locator("#username").fill(account.username);
  await page.locator("#password").fill(account.password);
  await page.locator("#kc-login").click();
  await page.waitForURL(play);
}

// Roles land in the access token at sign-in, so an account gets its groups before it first signs in:
// created straight through Keycloak's admin API, as the bootstrap admin only a test stack has.
export async function provisionAccount(page: Page, groups: KeycloakGroup[]): Promise<Account> {
  const account = newAccount();
  const admin = {
    username: requireEnv("KC_BOOTSTRAP_ADMIN_USERNAME"),
    password: requireEnv("KC_BOOTSTRAP_ADMIN_PASSWORD"),
  };
  const user = {
    username: account.username,
    email: email(account),
    firstName: account.username,
    lastName: account.username,
    enabled: true,
    emailVerified: true,
    credentials: [{ type: "password", value: account.password, temporary: false }],
    groups: groups.map((group) => `/${group}`),
  };
  // Through a page on Keycloak's own origin, so the request resolves the stack's domain exactly as
  // the browser does and needs no CORS.
  const keycloak = await page.context().newPage();
  try {
    await keycloak.goto("/");
    const site = new URL(keycloak.url());
    await keycloak.goto(`${site.protocol}//auth.${site.host}/realms/master`);
    const status = await keycloak.evaluate(
      async ({ admin, user, realm }) => {
        const token = await fetch("/realms/master/protocol/openid-connect/token", {
          method: "POST",
          body: new URLSearchParams({ grant_type: "password", client_id: "admin-cli", ...admin }),
        }).then((response) => response.json());
        const created = await fetch(`/admin/realms/${realm}/users`, {
          method: "POST",
          headers: { Authorization: `Bearer ${token.access_token}`, "Content-Type": "application/json" },
          body: JSON.stringify(user),
        });
        return created.status;
      },
      { admin, user, realm: REALM },
    );
    if (status !== 201) throw new Error(`provisioning ${account.username} failed: HTTP ${status}`);
  } finally {
    await keycloak.close();
  }
  return account;
}


function newAccount(): Account {
  counter += 1;
  // Lowercase alphanumerics (Keycloak's username rules), unique across parallel workers and retries.
  const username = `tester${Date.now().toString(36)}${process.pid.toString(36)}${counter}`;
  return { username, password: `Passw0rd-${username}` };
}

function email(account: Account): string {
  return `${account.username}@example.com`;
}

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is not set`);
  return value;
}

async function fillIfPresent(page: Page, selector: string, value: string): Promise<void> {
  const field = page.locator(selector);
  if ((await field.count()) > 0) {
    await field.fill(value).catch(() => {});
  }
}
