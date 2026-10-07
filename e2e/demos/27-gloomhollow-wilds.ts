import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, hunt, walkAlong } from "../helpers/tour";

test(
  "Gloomhollow's wilds",
  chapter({
    summary: "Ghouls among the torn graves and the abbey ruins",
    setup: (page) => arriveSeasoned(page, "Gloomhollow", [27.5, 30]),
    play: async (page) => {
      await caption(page, "South of the street, the gallows field: torn graves round the gallows tree");
      await walkAlong(page, [[26, 38], [20, 44]], 400);
      await caption(page, "Ghouls dig among the graves");
      await hunt(page, "Ghoul", "gallows-field", 1);
    },
  }),
);
