import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, hunt, walkAlong } from "../helpers/tour";

test(
  "Glimmercap's wilds",
  chapter({
    summary: "Myconids in the deep caps and glow moths in their glade",
    setup: (page) => arriveSeasoned(page, "Glimmercap", [40.5, 43]),
    play: async (page) => {
      await caption(page, "Past the vine bridge, myconids roam the deep caps where the cap-cutters' camp was abandoned");
      await walkAlong(page, [[48, 38], [53, 36]], 400);
      await hunt(page, "Myconid", "deep-caps", 1);
      await caption(page, "Glow moths drift over the flower meadow in the glade to the south");
      await walkAlong(page, [[40, 44], [28, 46]], 400);
      await hunt(page, "GlowMoth", "moth-glade", 1);
    },
  }),
);
