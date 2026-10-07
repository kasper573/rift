import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, hunt, walkAlong } from "../helpers/tour";

test(
  "Sunscar's wilds",
  chapter({
    summary: "Scorpions in the rocks east of town, and the dead walking in the ruins",
    setup: (page) => arriveSeasoned(page, "Sunscar", [52.5, 27.5]),
    play: async (page) => {
      await caption(page, "The east road leaves town between the rocks and the old ruins");
      await walkAlong(page, [[56, 25.5]], 400);
      await caption(page, "Scorpions shelter in the rocks; they sting only when provoked");
      await hunt(page, "Scorpion", "scorpion-rocks", 1);
      await caption(page, "To the south lie the ruins of an older town, where the dead walk in their wrappings");
      await walkAlong(page, [[57.5, 30], [57.5, 37]], 400);
      await hunt(page, "SandMummy", "ruins", 2);
    },
  }),
);
