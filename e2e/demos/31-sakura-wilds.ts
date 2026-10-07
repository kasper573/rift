import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, hunt, walkAlong } from "../helpers/tour";

test(
  "Sakura's wilds",
  chapter({
    summary: "Fox spirits in the northern wood and ronin in the bamboo",
    setup: (page) => arriveSeasoned(page, "Sakura", [46, 40]),
    play: async (page) => {
      await caption(page, "The torii path climbs north into the fox wood");
      await walkAlong(page, [[47, 34], [47, 27]], 400);
      await hunt(page, "Kitsune", "fox-wood", 1);
      await caption(page, "Masterless ronin camp in the southern bamboo");
      await walkAlong(page, [[36, 50], [36, 58]], 400);
      await hunt(page, "Ronin", "bamboo-grove", 1);
    },
  }),
);
