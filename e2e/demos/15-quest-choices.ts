import { test } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { admin } from "../helpers/admin";
import { caption, chapter } from "../helpers/demo";
import {
  clickUi,
  doubleClickUi,
  finished,
  focusGame,
  holding,
  hoverUi,
  onQuest,
  probe,
  travelTo,
  waitFor,
  waitForWorld,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, interactWith, onStage, pick, readToChoices, talkTo } from "../helpers/talk";

const MAP = "icons/misc/map.png";

test(
  "Quest choices",
  chapter({
    summary: "A quest found in a chest, a place to explore, and a choice you can't take back",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await admin(page, [
        ["/give OrcTusk,5", /gave 5 Orc Tusk/],
        ["/give RoadPass,1", /gave 1 Road Pass/],
        ["/xp 40", /granted 40 xp/],
        ["/quest TusksForTheChief,accept", /TusksForTheChief: Accept/],
      ]);
    },
    play: async (page) => {
      await caption(page, "The tide chest hides more than gold");
      await interactWith(page, "TideChest");
      await onStage(page, "TideChestMap");
      await readToChoices(page, 1400);
      await pick(page, "Close the lid.");
      await conversationOver(page);
      await waitFor(page, (snapshot) => holding(snapshot, "TatteredMap") === 1, "the map never arrived");

      await caption(page, "The map begins a quest — double-click it in your bag to use it", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("KeyI");
      await hoverUi(page, MAP);
      await page.waitForTimeout(2500);
      await doubleClickUi(page, MAP);
      await onStage(page, "XMarksOffer");
      await caption(page, "No one to talk to: just you, and the map", { at: "top" });
      await readToChoices(page, 1400);
      await page.waitForTimeout(1500);
      await pick(page, "Follow the map.");
      await waitFor(page, (snapshot) => onQuest(snapshot, "XMarksTheSpot"), "the map's quest never started");
      await focusGame(page);
      await page.keyboard.press("KeyI");

      await caption(page, "The cross is in the forest");
      const road = (await probe(page)).portals.find((portal) => portal.name === "forest-road")!;
      await travelTo(page, road.at);
      await waitFor(page, ({ area }) => area === "Forest", "never reached the forest");
      await page.waitForTimeout(1500);

      await caption(page, "Finding the standing stone completes the objective");
      await travelTo(page, ({ markers }) => markers.find((marker) => marker.name === "standing-stone")?.at, 120_000);
      await interactWith(page, "StandingStone");
      await waitFor(page, (snapshot) => onQuest(snapshot, "XMarksTheSpot")?.ready, "the stone was never found");
      await readToChoices(page, 1000);
      await pick(page, "X Marks the Spot");
      await onStage(page, "XMarksThanks");
      await caption(page, "A map object takes the hand-in, like any townsperson", { at: "top" });
      await readToChoices(page, 1200);
      await page.waitForTimeout(1500);
      await pick(page, "Dig where the cross says.");
      await waitFor(page, (snapshot) => finished(snapshot, "XMarksTheSpot") === "Completed", "never dug it up");
      await page.waitForTimeout(2500);

      await caption(page, "Ugra, the clan's shaman, knows about Mara's errand");
      await travelTo(page, ({ markers }) => markers.find((marker) => marker.name === "ugra-camp")?.at, 120_000);
      await talkTo(page, "Ugra");
      await readToChoices(page, 1000);
      await pick(page, "The Shaman's Plea");
      await onStage(page, "PleaOffer");
      await readToChoices(page, 1400);
      await pick(page, "I'm listening.");
      await onStage(page, "PleaAnswer");
      await caption(page, "Both answers are permanent, so this one shows everything: what it needs, what it costs, and a warning", {
        at: "top",
      });
      await readToChoices(page, 1400);
      await hoverUi(page, "Give her the tusks.");
      await page.waitForTimeout(4000);
      await pick(page, "Give her the tusks.");
      await onStage(page, "UgraGrateful");
      await waitFor(page, (snapshot) => finished(snapshot, "TusksForTheChief") === "Failed", "Mara's quest held");
      await waitFor(
        page,
        ({ feed }) => feed.includes("Quest failed · Tusks for the Chief"),
        "the failed quest never showed in the feed",
      );
      await caption(page, "Mara's quest fails, in red in the corner, and the choice is remembered", { at: "top" });
      await readToChoices(page, 1400);
      await pick(page, "Rest easy, Ugra.");
      await conversationOver(page);

      await caption(page, "Now Ugra greets a friend of the clan, with work and remedies of her own");
      await talkTo(page, "Ugra");
      await onStage(page, "UgraFriend");
      await readToChoices(page, 1000);
      await page.waitForTimeout(3500);
      await focusGame(page);
      await page.keyboard.press("Escape");
      await conversationOver(page);
    },
  }),
);
