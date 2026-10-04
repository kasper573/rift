import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { closestTile, probe, walkTo, type Tile } from "../helpers/game";

const ROUTE: Array<{ from_spawn: Tile; caption: string }> = [
  { from_spawn: [4, 1], caption: "Click the ground to walk there — the camera follows" },
  { from_spawn: [5, -2], caption: "Paths route around water, rocks and anything in the way" },
  { from_spawn: [-3, -4], caption: "Beaches, docks and palm groves — the island is open to explore" },
  { from_spawn: [0, 0], caption: "Monsters roam freely and fight back" },
];

test(
  "Explore",
  chapter({
    summary: "Point and click to roam the island",
    play: async (page) => {
      const spawn = (await probe(page)).me!.at;
      for (const stop of ROUTE) {
        await caption(page, stop.caption);
        const { walkable } = await probe(page);
        await walkTo(page, closestTile(walkable, [spawn[0] + stop.from_spawn[0], spawn[1] + stop.from_spawn[1]])!);
        await page.waitForTimeout(1500);
      }
    },
  }),
);
