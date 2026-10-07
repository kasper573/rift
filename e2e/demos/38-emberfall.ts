import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, enter, meet, walkAlong } from "../helpers/tour";

test(
  "Emberfall",
  chapter({
    summary: "A dwarf hold where a lava river comes out of the mountain: the forge and the fire shrine",
    setup: (page) => arriveSeasoned(page, "Emberfall", [2.5, 30.5]),
    play: async (page) => {
      await caption(page, "Emberfall: a dwarf hold at the end of the desert road, where a lava river leaves the mountain");
      await walkAlong(page, [[12, 31], [24, 31]], 400);
      await caption(page, "The forge hall has two doors: one to the smithy, one to the living hall");
      await enter(page, "forge-smithy");
      await caption(page, "Furnace under the chimney, bellows, anvil and quench tub; the living hall through the partition");
      await meet(page, "Blacksmith", 3500);
      await enter(page, "forge-door");
      await caption(page, "The fire shrine stands on the lava's bank");
      await enter(page, "fire-shrine");
      await caption(page, "The eternal flame burns on the altar between the ancestor statues");
      await meet(page, "FirePriestess", 3500);
      await enter(page, "front-door");
      await caption(page, "A dwarf miner keeps watch at the gap in the barricade across the mine road");
      await meet(page, "DwarfMiner");
    },
  }),
);
