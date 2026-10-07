import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, hunt, walkAlong } from "../helpers/tour";

test(
  "Frostmere's wilds",
  chapter({
    summary: "Wolves in the woods past the log bridge, and wraiths at the barrow",
    setup: (page) => arriveSeasoned(page, "Frostmere", [35.5, 21]),
    play: async (page) => {
      await caption(page, "Past the huntress's door a trail crosses the mere's outflow on a log bridge, out of the hamlet's safety");
      await walkAlong(page, [[40.5, 25.5], [45, 27.5], [52, 27.5]], 400);
      await caption(page, "Dire wolves hunt the woods in packs");
      await hunt(page, "DireWolf", "wolf-woods", 2);
      await caption(page, "The hamlet buries its dead in the southwest hollow");
      await walkAlong(page, [[45, 27.5], [36, 21.5], [20, 21], [8, 21], [7.5, 24.5]]);
      await caption(page, "Frost wraiths drift among the barrow stones");
      await hunt(page, "FrostWraith", "barrow", 2);
    },
  }),
);
