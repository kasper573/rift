import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, enter, meet, walkAlong } from "../helpers/tour";

test(
  "Bloomvale",
  chapter({
    summary: "A farm village at the crossroads: the windmill, the baker's cottage and the inn",
    setup: (page) => arriveSeasoned(page, "Forest", [34.5, 8.5]),
    play: async (page) => {
      await caption(page, "A road leaves the forest to the east, down into the farmlands");
      await walkAlong(page, [[40, 7.5]], 400);
      await enter(page, "bloomvale-road");
      await caption(page, "Bloomvale: a farm village where the four roads meet, on the west bank of a brook");
      await walkAlong(page, [[12, 32], [22, 32]], 400);
      await caption(page, "A farmer girl tends the vegetable plots and the pumpkin patch south of the road");
      await meet(page, "FarmerGirl");
      await caption(page, "The baker's cottage sits at the foot of the mill lane");
      await enter(page, "baker-cottage");
      await caption(page, "Bread oven, flour and the rising dough by the door; her bedroom behind");
      await meet(page, "Baker", 3500);
      await enter(page, "front-door");
      await caption(page, "Up the lane the windmill turns between the cut wheat and the standing corn");
      await enter(page, "mill");
      await caption(page, "Grain comes in on one side of the millstones and goes out as flour on the other");
      await meet(page, "Miller", 3500);
      await enter(page, "front-door");
      await caption(page, "The inn stands at the crossroads, with the well in the square before it");
      await walkAlong(page, [[30, 31], [41, 30.5]], 400);
      await enter(page, "inn-taproom");
      await caption(page, "Three doors lead in: the taproom, the kitchen and the stable, all joined inside");
      await walkAlong(page, [[12, 12], [20, 10]], 600);
      await enter(page, "taproom-door");
    },
  }),
);
