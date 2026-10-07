import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, hunt, walkAlong } from "../helpers/tour";

test(
  "Mirefen's wilds",
  chapter({
    summary: "Bog lurkers in the eastern fen and slimes in the old peat cuttings",
    setup: (page) => arriveSeasoned(page, "Mirefen", [56, 25]),
    play: async (page) => {
      await caption(page, "East over the footbridge, past an abandoned goat shack, the trail leads into the lurker fen");
      await walkAlong(page, [[62, 27], [68, 30]], 400);
      await hunt(page, "BogLurker", "lurker-fen", 1);
      await caption(page, "South of the outflow, slimes ooze through the abandoned peat cuttings");
      await walkAlong(page, [[56, 35], [56, 42], [40, 44]], 400);
      await hunt(page, "SwampSlime", "slime-bog", 1);
    },
  }),
);
