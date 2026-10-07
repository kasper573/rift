import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, enter, meet, walkAlong } from "../helpers/tour";

test(
  "Glimmercap",
  chapter({
    summary: "A glowing grove round a warm spring: the druid's tree, the fairy's mushroom and the gnome's yard",
    setup: (page) => arriveSeasoned(page, "Glimmercap", [2.5, 30.5]),
    play: async (page) => {
      await caption(page, "Glimmercap: a grove in a hollow of deep woods, round a warm and glowing spring");
      await walkAlong(page, [[12, 30], [22, 30]], 400);
      await caption(page, "The druid lives in the ancient tree at the head of the pool");
      await enter(page, "hollow-tree");
      await caption(page, "His workroom in front, where he keeps herbs, potions and seeds; he sleeps in the back");
      await meet(page, "Druid", 3500);
      await enter(page, "front-door");
      await caption(page, "The fairy's red mushroom stands among her flower beds to the southwest");
      await enter(page, "mushroom-house");
      await meet(page, "Fairy", 3500);
      await enter(page, "front-door");
      await caption(page, "The gnome alchemist works the yard between his burrow and his workshop at the outflow");
      await walkAlong(page, [[24, 42], [33, 42]]);
      await meet(page, "GnomeAlchemist");
    },
  }),
);
