import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, enter, meet, walkAlong } from "../helpers/tour";

test(
  "Amberwood",
  chapter({
    summary: "A woodcutters' hamlet and a farm in the autumn forest",
    setup: (page) => arriveSeasoned(page, "Amberwood", [31.5, 81.5]),
    play: async (page) => {
      await caption(page, "Amberwood: the Bloomvale road climbs into the autumn forest past a farm");
      await walkAlong(page, [[33, 72], [40, 68]], 400);
      await caption(page, "The red barn has two doors: the big doors to the threshing floor and one to the stable");
      await enter(page, "barn-doors");
      await caption(page, "Hay, the loft ladder and the plough; through the partition, the farmhand's stall");
      await meet(page, "Farmhand", 3500);
      await enter(page, "barn-doors");
      await caption(page, "In the hamlet, the old woodsman's cabin stands by his hives and his late wife's grave");
      await walkAlong(page, [[31, 55], [25, 49]], 400);
      await enter(page, "woodsman-cabin");
      await caption(page, "One room, arranged by use: the hearth facing the door, kitchen at one end, bed at the other");
      await meet(page, "OldWoodsman", 3500);
      await enter(page, "front-door");
      await caption(page, "The lumberjack works his yard where the road forks east for Frostmere");
      await meet(page, "Lumberjack");
    },
  }),
);
