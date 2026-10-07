import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, hunt, walkAlong } from "../helpers/tour";

test(
  "Verdant's wilds",
  chapter({
    summary: "Panthers across the river and lizardmen in the sunken city",
    setup: (page) => arriveSeasoned(page, "Verdant", [30, 29]),
    play: async (page) => {
      await caption(page, "Panthers have taken the far bank, where the hunters' trail ends at their abandoned blind");
      await walkAlong(page, [[38, 29], [44, 28]], 400);
      await hunt(page, "Panther", "panther-jungle", 1);
      await caption(page, "Beyond the overgrown bridge, lizardmen hold the outer city, sunk into the spring pools");
      await walkAlong(page, [[36, 30], [30, 60], [40, 62]], 400);
      await hunt(page, "Lizardman", "lizard-ruins", 1);
    },
  }),
);
