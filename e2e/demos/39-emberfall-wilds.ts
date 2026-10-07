import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, hunt, walkAlong } from "../helpers/tour";

test(
  "Emberfall's wilds",
  chapter({
    summary: "Lava golems at the abandoned mine and magma imps in the lava fields",
    setup: (page) => arriveSeasoned(page, "Emberfall", [45, 22]),
    play: async (page) => {
      await caption(page, "Past the barricade, the golems came up from a lava pool by the mine's cart track");
      await walkAlong(page, [[44, 17]], 400);
      await hunt(page, "LavaGolem", "mine-yard", 1);
      await caption(page, "East over the black bridge, magma imps swarm the lava fields");
      await walkAlong(page, [[45, 31], [56, 33], [64, 33]], 400);
      await hunt(page, "MagmaImp", "lava-fields", 1);
    },
  }),
);
