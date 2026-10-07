import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, enter, meet, walkAlong } from "../helpers/tour";

test(
  "Verdant",
  chapter({
    summary: "A jungle village by a river: the shaman's hut and the vine temple",
    setup: (page) => arriveSeasoned(page, "Verdant", [21.5, 2.5]),
    play: async (page) => {
      await caption(page, "Verdant: a jungle village on the west bank of a river that rises in springs below a ridge");
      await walkAlong(page, [[19, 14], [20, 26]], 400);
      await caption(page, "The shaman's hut, with ancestor posts by the door");
      await enter(page, "shaman-hut");
      await caption(page, "One bamboo room facing the ancestor altar, the healing corner under the window");
      await meet(page, "Shaman", 3500);
      await enter(page, "front-door");
      await caption(page, "South, the old city's processional way runs east from the vine temple");
      await walkAlong(page, [[18, 45], [14, 58]], 400);
      await enter(page, "vine-temple");
      await caption(page, "The archaeologist camps in the vestibule; beyond the wall, the untouched sanctum");
      await meet(page, "Archaeologist", 3500);
      await enter(page, "front-door");
      await caption(page, "An explorer has pitched camp at the crossing");
      await meet(page, "Explorer");
    },
  }),
);
