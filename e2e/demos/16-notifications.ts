import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { admin } from "../helpers/admin";
import { caption, chapter } from "../helpers/demo";
import {
  arrivedIn,
  clickUi,
  closestTile,
  doubleClickUi,
  focusGame,
  historyTab,
  holding,
  hoverUi,
  nearest,
  probe,
  travelTo,
  waitFor,
  waitForWorld,
  walkInto,
  walkTo,
  type Snapshot,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, onStage, pick, readToChoices, talkTo } from "../helpers/talk";

const BAG = "icons/equipment/bag.png";
const HELMET = "icons/equipment/leather_helmet.png";
const CUTLASS = "icons/weapon_and_tool/golden_sword.png";

test(
  "Notifications",
  chapter({
    summary: "Every notification where it belongs: bubbles, captions, alerts, errors, milestones and intros",
    setup: async (page) => {
      await signIn(page, await provisionAccount(page, ["admin"]));
      await clickUi(page, "Play");
      await waitForWorld(page, loadReference("island.png"));
      await admin(page, [
        ["/give FishSteak,1", /gave 1 Fish Steak/],
        ["/give TribalHelmet,1", /gave 1 Tribal Helmet/],
        ["/give CorsairCutlass,1", /gave 1 Corsair's Cutlass/],
        ["/xp 40", /granted 40 xp/],
      ]);
    },
    play: async (page) => {
      await caption(page, "Some places start a conversation on their own: Ilsa watches the forest road");
      const road = (await probe(page)).portals.find((portal) => portal.name === "forest-road")!;
      await travelTo(page, road.at);
      await onStage(page, "ilsa_halt");
      await caption(page, "Without a pass the road is just ground. Standing on it, she stops you and your commands lock", {
        at: "top",
      });
      await readToChoices(page, 1400);
      await caption(page, "Ilsa shows only what she means to: the road opens at level 3", { at: "top" });
      await hoverUi(page, "I'm ready for the forest road.");
      await page.waitForTimeout(2500);
      await pick(page, "I'll turn back.");
      await conversationOver(page);

      await caption(page, "Talk to her, though, and a fish changes her mind");
      await talkTo(page, "ilsa");
      await readToChoices(page, 1000);
      await hoverUi(page, "Would a fish change your mind?");
      await page.waitForTimeout(2000);
      await pick(page, "Would a fish change your mind?");
      await onStage(page, "ilsa_fish");
      await waitFor(page, ({ feed }) => feed.includes("+1 Road Pass"), "the pass never showed in the feed");
      await caption(page, "The trade shows in the corner, over her bust: the fish goes, the pass arrives", { at: "top" });
      await readToChoices(page, 1800);
      await pick(page, "Enjoy it.");
      await conversationOver(page);
      await waitFor(page, (snapshot) => holding(snapshot, "road_pass") === 1, "the pass never arrived");

      await caption(page, "Narration plays as captions at the top and never blocks: gulls squabble at the pier");
      await walkInto(page, "ferry-pier");
      await waitFor(page, (snapshot) => captioned(snapshot, "Gulls"), "the gulls never squabbled");
      await page.waitForTimeout(1000);
      await caption(page, "Hovering holds the captions, so you can finish reading", { at: "top" });
      await hoverUi(page, /squabble/);
      await page.waitForTimeout(3500);
      await caption(page, "Every line is kept in history as well — H shows it", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("KeyH");
      await waitFor(page, ({ history }) => history.open, "history never opened");
      await page.waitForTimeout(3000);
      await page.keyboard.press("KeyH");
      await waitFor(page, ({ history }) => !history.open, "history never closed");

      await caption(page, "What you can't do is an error, in red at the top: the Tribal Helmet wants level 3");
      await clickUi(page, BAG);
      await doubleClickUi(page, HELMET);
      await waitFor(page, (snapshot) => refused(snapshot, "Tribal Helmet"), "the helmet was never refused");
      await page.waitForTimeout(1500);
      await caption(page, "Only the latest error shows: the Corsair's Cutlass wants level 3 too", { at: "top" });
      await doubleClickUi(page, CUTLASS);
      await waitFor(page, (snapshot) => refused(snapshot, "Corsair's Cutlass"), "the cutlass never replaced the helmet");
      await page.waitForTimeout(1500);
      await caption(page, "Trying again keeps that one error up, with nothing piling on", { at: "top" });
      await doubleClickUi(page, CUTLASS);
      await page.waitForTimeout(1500);
      await caption(page, "Errors it replaced are kept in history", { at: "top" });
      await focusGame(page);
      await page.keyboard.press("KeyH");
      await waitFor(page, ({ history }) => history.open, "history never opened");
      await openTab(page, "Errors");
      await waitFor(
        page,
        ({ history }) => history.records.filter(({ topic }) => topic === "Error").length >= 2,
        "history never kept both errors",
      );
      await page.waitForTimeout(3000);
      await focusGame(page);
      await page.keyboard.press("KeyH");
      await waitFor(page, ({ history }) => !history.open, "history never closed");
      await page.keyboard.press("KeyI");

      await caption(page, "Bram sails for twenty Gold, or for a pass he only needs to see");
      await talkTo(page, "bram");
      await readToChoices(page, 1000);
      await hoverUi(page, "Ilsa gave me this pass.");
      await page.waitForTimeout(2500);
      await pick(page, "Ilsa gave me this pass.");
      await waitFor(page, (snapshot) => arrivedIn(snapshot, "forest"), "the Gull never sailed");
      await caption(page, "The crossing narrates itself while you walk on", { at: "top" });
      await waitFor(page, (snapshot) => captioned(snapshot, "The Gull"), "the crossing was never told");
      await stroll(page, [3, 1]);
      await caption(page, "A first visit opens on the area's own title, to its own drums", { at: "top" });
      await waitFor(page, ({ notifications }) => notifications.intro?.title === "The forest", "the forest never spoke", 15_000);
      await stroll(page, [-2, 2]);
      await page.waitForTimeout(3000);

      await caption(page, "(Admin shortcut) An alert reaches every player, in a bar of its own");
      await admin(page, [["/notify The last ferry back leaves at dusk.", /notified every area/]]);
      await waitFor(page, ({ notifications }) => notifications.alerts.rows.length > 0, "the alert never arrived");
      await page.waitForTimeout(2500);
      await caption(page, "It stays until its time is up, or until you close it", { at: "top" });
      await clickUi(page, "\u00d7");
      await waitFor(page, ({ notifications }) => notifications.alerts.rows.length === 0, "the alert never closed");
      await page.waitForTimeout(1200);

      await caption(page, "(Admin shortcut: experience) A new level is a milestone, in the middle of the screen");
      await admin(page, [["/xp 300", /granted 300 xp/]]);
      await waitFor(
        page,
        ({ notifications }) => notifications.milestones.rows.some(({ label }) => label === "Level up"),
        "the level never showed",
      );
      await page.waitForTimeout(3000);

      await caption(page, "(Admin shortcut: on Mara's errand, with an Orc Chief close by)", { at: "top" });
      await admin(page, [["/quest TusksForTheChief,accept", /TusksForTheChief: Accept/]]);
      await waitFor(
        page,
        ({ feed }) => feed.includes("Quest accepted · Tusks for the Chief"),
        "the accepted quest never showed in the feed",
      );
      await caption(page, "Your own news slides quietly into the corner");
      await page.waitForTimeout(2500);
      await admin(page, [["/spawn OrcChief", /spawned Orc Chief/]]);
      await waitFor(
        page,
        ({ notifications }) => notifications.bubbles.some(({ npc }) => npc === "orc_chief"),
        "the Orc Chief never yelled",
      );
      await caption(page, "Voices speak from where they stand: a bubble over the chief, in his own voice");
      await page.waitForTimeout(2500);
      await caption(page, "You keep fighting: no tether, no lock, and your foe's bubble stays up front");
      await fight(page, "orc_chief", 9000);
      await page.waitForTimeout(1500);
    },
  }),
);

function captioned({ notifications }: Snapshot, label: string): boolean {
  return notifications.captions.rows.some((row) => row.label === label);
}

function refused({ notifications }: Snapshot, item: string): boolean {
  return notifications.error?.startsWith(item) ?? false;
}

async function openTab(page: Page, title: string): Promise<void> {
  const tab = await historyTab(page, title);
  await clickUi(page, (element) => element.text === tab.text && element.x === tab.x && element.y === tab.y);
  await waitFor(page, ({ history }) => history.tab === title, `the ${title} tab never opened`);
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
