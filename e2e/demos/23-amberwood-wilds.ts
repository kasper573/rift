import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, hunt, walkAlong } from "../helpers/tour";

test(
  "Amberwood's wilds",
  chapter({
    summary: "Goblins camped across the footbridge, and bears in the northern woods",
    setup: (page) => arriveSeasoned(page, "Amberwood", [30, 58]),
    play: async (page) => {
      await caption(page, "Across the farm's footbridge, goblins have made camp with the farm's stolen pumpkins");
      await walkAlong(page, [[20, 58], [14, 58]], 400);
      await hunt(page, "Goblin", "goblin-woods", 1);
      await caption(page, "The felling trail north ends at a clearing abandoned in a hurry: the bears' woods");
      await walkAlong(page, [[24, 40], [25, 30]], 400);
      await hunt(page, "BrownBear", "bear-woods", 1);
    },
  }),
);
