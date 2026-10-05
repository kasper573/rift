import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { admin } from "../helpers/admin";
import { caption, chapter } from "../helpers/demo";
import {
  clickUi,
  closestTile,
  focusGame,
  holding,
  hoverUi,
  nearest,
  probe,
  travelTo,
  waitFor,
  waitForWorld,
  walkTo,
  type Snapshot,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, onStage, pick, readToChoices, talkTo } from "../helpers/talk";

test(
  "Announcements",
  chapter({
    summary: "Places that stop you, lines that find you, and a ferry to the forest",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await admin(page, [
        ["/give FishSteak,1", /gave 1 Fish Steak/],
        ["/xp 40", /granted 40 xp/],
      ]);
    },
    play: async (page) => {
      await caption(page, "Some places start a conversation on their own: Ilsa watches the forest road");
      const { portals, markers } = await probe(page);
      const gate = markers.find((marker) => marker.name === "forest-gate")!.at;
      const road = nearest(
        portals.filter((portal) => portal.to === "Forest"),
        gate,
      )!;
      await travelTo(page, road.at);
      await onStage(page, "IlsaHalt");
      await caption(page, "Heading for the road walked you into her zone: your walk stops and your commands lock", {
        at: "top",
      });
      await readToChoices(page, 1400);
      await caption(page, "No pass, no Gold — turning back is all that's open", { at: "top" });
      await hoverUi(page, "I have a Road Pass.");
      await page.waitForTimeout(2500);
      await pick(page, "I'll turn back.");
      await conversationOver(page);

      await caption(page, "Talk to her, though, and a fish changes her mind");
      await talkTo(page, "Ilsa");
      await readToChoices(page, 1000);
      await hoverUi(page, "Would a fish change your mind?");
      await page.waitForTimeout(2000);
      await pick(page, "Would a fish change your mind?");
      await onStage(page, "IlsaFish");
      await readToChoices(page, 1200);
      await pick(page, "Enjoy it.");
      await conversationOver(page);
      await waitFor(page, (snapshot) => holding(snapshot, "RoadPass") === 1, "the pass never arrived");

      await caption(page, "Announcements play at the top and never block: the harbour bell rings at the pier");
      await travelTo(page, ({ markers }) => markers.find((marker) => marker.name === "ferry-pier")?.at);
      await waitFor(page, (snapshot) => showing(snapshot, "HarbourBell"), "the harbour bell never rang");
      await page.waitForTimeout(1500);
      await caption(page, "A line about the gulls waits behind it, but only for three seconds", { at: "top" });
      await waitFor(page, ({ announcement }) => announcement.missed.includes("Gulls"), "the gulls never gave up");
      await page.waitForTimeout(1500);
      await caption(page, "Lines that wait too long go straight to history — H shows it", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("KeyH");
      await waitFor(page, ({ history }) => history, "history never opened");
      await page.waitForTimeout(3500);
      await page.keyboard.press("KeyH");

      await caption(page, "Bram sails for twenty Gold, or for a pass he only needs to see");
      await talkTo(page, "Bram");
      await readToChoices(page, 1000);
      await hoverUi(page, "Ilsa gave me this pass.");
      await page.waitForTimeout(2500);
      await pick(page, "Ilsa gave me this pass.");
      await waitFor(page, ({ area }) => area === "Forest", "the Gull never sailed");
      await caption(page, "The crossing and the forest narrate themselves while you walk on", { at: "top" });
      await waitFor(page, (snapshot) => showing(snapshot, "GullSails"), "the crossing was never told");
      await stroll(page, [3, 1]);
      await waitFor(page, (snapshot) => showing(snapshot, "ForestArrival"), "the forest never spoke", 15_000);
      await stroll(page, [-2, 2]);
      await page.waitForTimeout(2500);

      await caption(page, "(Admin shortcut: on Mara's errand, with an Orc Chief close by)", { at: "top" });
      await admin(page, [
        ["/quest TusksForTheChief,accept", /TusksForTheChief: Accept/],
        ["/spawn OrcChief", /spawned Orc Chief/],
      ]);
      await waitFor(
        page,
        (snapshot) => showing(snapshot, "ChiefChallenge") || showing(snapshot, "ChiefThreat"),
        "the Orc Chief never yelled",
      );
      await caption(page, "A boss yell is urgent and jumps the queue. You keep fighting: no tether, no lock");
      await fight(page, "OrcChief", 9000);
      await page.waitForTimeout(1500);
    },
  }),
);

function showing({ announcement }: Snapshot, id: string): boolean {
  return announcement.showing === id;
}

async function stroll(page: Page, [dx, dy]: [number, number]): Promise<void> {
  const { me, walkable } = await probe(page);
  await walkTo(page, closestTile(walkable, [me!.at[0] + dx, me!.at[1] + dy])!);
}

async function fight(page: Page, npc: string, forMs: number): Promise<void> {
  const until = Date.now() + forMs;
  while (Date.now() < until) {
    const foe = ({ me, actors }: Snapshot) =>
      me ? nearest(actors.filter((actor) => actor.npc === npc && actor.health > 0), me.at)?.aim : undefined;
    await travelTo(page, foe).catch(() => undefined);
    await page.waitForTimeout(1500);
  }
}
