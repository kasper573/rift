import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, enter, meet, walkAlong } from "../helpers/tour";

test(
  "Sakura",
  chapter({
    summary: "A blossom valley: the teahouse, the lord's estate and the temple",
    setup: (page) => arriveSeasoned(page, "Sakura", [2.5, 45]),
    play: async (page) => {
      await caption(page, "Sakura: a blossom valley, its stream bridged at one point by the red bridge");
      await caption(page, "The teahouse stands on the west bank, with its parasol, bench and sign");
      await enter(page, "teahouse");
      await caption(page, "A guest room round the sunken hearth, and a preparation room behind the partition");
      await meet(page, "TeaMaster", 3500);
      await enter(page, "front-door");
      await caption(page, "Across the bridge, the lord's estate holds the crossing");
      await walkAlong(page, [[18, 45], [27, 43]], 400);
      await enter(page, "estate-gate");
      await caption(page, "Seven rooms: entrance hall, audience hall, shrine, quarters, tea room, kitchen and stores");
      await meet(page, "SamuraiLord", 3500);
      await enter(page, "gate-door");
      await caption(page, "At the east end a monk keeps vigil by the temple and the torii path up to the fox wood");
      await meet(page, "Monk");
      await enter(page, "temple");
      await caption(page, "The temple keeps the lord's grandfather's armour, and the monk's sleeping corner behind a screen");
      await walkAlong(page, [[8, 6]], 1500);
      await enter(page, "front-door");
    },
  }),
);
