import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, enter, meet, walkAlong } from "../helpers/tour";

test(
  "Frostmere",
  chapter({
    summary: "A hunting hamlet on the shore of a frozen mere: the jarl's hall and the huntress's cabin",
    setup: (page) => arriveSeasoned(page, "Frostmere", [2, 21]),
    play: async (page) => {
      await caption(page, "Frostmere: a hunting hamlet on the shore of a frozen mere, at the end of the west road");
      await walkAlong(page, [[5, 21]]);
      await caption(page, "A fur trapper keeps his post where the barrow path turns off the road");
      await meet(page, "Trapper");
      await caption(page, "The jarl's longhouse faces the plaza, with its runestone and the well that never freezes");
      await walkAlong(page, [[13, 21], [19.5, 19]], 600);
      await enter(page, "jarl-longhouse");
      await caption(page, "Inside, the jarl holds court on his high seat between the long fires");
      await meet(page, "Jarl", 3500);
      await enter(page, "front-door");
      await caption(page, "The road runs on to the huntress's cabin, between the mere and the wolf woods");
      await walkAlong(page, [[26, 21], [33, 21]], 400);
      await enter(page, "huntress-cabin");
      await caption(page, "She lives alone at the edge of things, by her hearth and her furs");
      await meet(page, "Huntress", 3500);
      await enter(page, "front-door");
    },
  }),
);
