import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, enter, meet, walkAlong } from "../helpers/tour";

test(
  "Sunscar",
  chapter({
    summary: "An oasis town in the desert: the potter, the caravan trader and the silk prince",
    setup: (page) => arriveSeasoned(page, "Sunscar", [16.5, 2.5]),
    play: async (page) => {
      await caption(page, "Sunscar: an oasis town, reached through a pass in the northern rocks");
      await walkAlong(page, [[16.5, 9], [19, 17]], 400);
      await caption(page, "The potter's house has two doors: one into her home, one into her workshop");
      await walkAlong(page, [[21.5, 15.5]]);
      await enter(page, "potter-home");
      await caption(page, "Her home is on one side of the wall, her kiln and wheel on the other");
      await walkAlong(page, [[5, 7], [12, 7], [15, 8]], 300);
      await meet(page, "Potter", 3500);
      await enter(page, "workshop-door");
      await caption(page, "A caravan trader camps by the fire south of the road");
      await meet(page, "NomadMerchant", 3500);
      await caption(page, "The silk prince's pavilion stands by the pool");
      await walkAlong(page, [[38, 20], [45.5, 15.5]], 400);
      await enter(page, "silk-pavilion");
      await caption(page, "He receives callers from his couch, among his bolts of silk");
      await meet(page, "SilkPrince", 3500);
      await enter(page, "front-door");
    },
  }),
);
