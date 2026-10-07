import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, enter, meet, walkAlong } from "../helpers/tour";

test(
  "Mirefen",
  chapter({
    summary: "An eel-fishing village on the one dry ridge through the fen",
    setup: (page) => arriveSeasoned(page, "Mirefen", [28, 2.5]),
    play: async (page) => {
      await caption(page, "Mirefen: an eel-fishing village on the one dry ridge through the fen");
      await walkAlong(page, [[31, 12], [36, 20]], 400);
      await caption(page, "The fisher family's stilt house stands with its back over the Eelmere");
      await enter(page, "fisher-stilt");
      await caption(page, "One plank room: the grandfather's bed by the stove, the boy's hammock, nets, rods and lures");
      await meet(page, "FisherBoy", 3500);
      await enter(page, "front-door");
      await caption(page, "An old eel fisher works the landing between the stilt houses");
      await meet(page, "EelFisher");
      await caption(page, "The witch lives west, across two pools on a straight boardwalk");
      await walkAlong(page, [[28, 22], [20, 24]], 400);
      await enter(page, "witch-hut");
      await caption(page, "Her workroom round the cauldron, her bedroom behind");
      await meet(page, "SwampWitch", 3500);
      await enter(page, "front-door");
    },
  }),
);
