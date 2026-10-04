import { test } from "@playwright/test";

import { register } from "../helpers/account";
import { waitForWorld } from "../helpers/game";
import { loadReference } from "../helpers/image";

test("a new player spawns into the island scene", async ({ page }) => {
  await register(page);
  await waitForWorld(page, loadReference("island.png"));
});
