import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, hunt, walkAlong } from "../helpers/tour";

test(
  "Bloomvale's wilds",
  chapter({
    summary: "Boars in the wild old orchard and bees in the abandoned apiary, past the field fence",
    setup: (page) => arriveSeasoned(page, "Bloomvale", [16.5, 40]),
    play: async (page) => {
      await caption(page, "A rail fence runs along the fields; the farm path leads through it to the wild old orchard");
      await walkAlong(page, [[16.5, 45], [15, 49]], 400);
      await caption(page, "Wild boars root in the old orchard and wallow in its mud; they fight only when provoked");
      await hunt(page, "WildBoar", "boar-orchard", 1);
      await caption(page, "Next door, giant bees took over the beekeeper's meadow when his hives were abandoned");
      await walkAlong(page, [[24, 50]], 400);
      await hunt(page, "GiantBee", "bee-meadow", 1);
    },
  }),
);
