import { test, type Page } from "@playwright/test";

import { provisionAccount, signIn } from "../helpers/account";
import { admin } from "../helpers/admin";
import { caption, chapter, inset } from "../helpers/demo";
import {
  clickUi,
  closestTile,
  focusGame,
  holding,
  hoverUi,
  nearest,
  occupied,
  probe,
  travelTo,
  waitFor,
  waitForWorld,
  waitUntilStill,
  type Snapshot,
} from "../helpers/game";
import { loadReference } from "../helpers/image";
import { conversationOver, onStage, pick, readToChoices, talkTo, townsperson } from "../helpers/talk";

let other: Page;

test(
  "Many players",
  chapter({
    summary: "One island, a world each: what you see of a townsperson is yours alone",
    setup: async (page, cast) => {
      other = await cast.newPage();
      for (const [player, commands] of [
        [page, [["/xp 40", /granted 40 xp/], ["/give Gold,300", /gave 300 Gold/]]],
        [
          other,
          [
            ["/xp 40", /granted 40 xp/],
            ["/quest TusksForTheChief,accept", /TusksForTheChief: Accept/],
          ],
        ],
      ] as [Page, [string, RegExp][]][]) {
        await signIn(player, await provisionAccount(player, ["admin"]));
        await clickUi(player, "Play");
        await waitForWorld(player, loadReference("island.png"));
        await admin(player, commands);
      }
      await standBelow(other, "Mara", 1);
      await standBelow(page, "Mara", -1);
    },
    play: async (page) => {
      const name = (await probe(other)).me!.name;
      const stop = await inset(page, other, `${name}'s screen`);
      try {
        await caption(page, `Two players at Mara's stall. On your screen she has a quest for you: !`);
        await waitFor(page, (snapshot) => townsperson(snapshot, "Mara")?.marks.includes("QuestOffered"), "no !");
        await page.waitForTimeout(3000);
        await caption(page, `On ${name}'s, top left, she shows a ?: they're halfway through it`);
        await waitFor(other, (snapshot) => townsperson(snapshot, "Mara")?.marks.includes("QuestInProgress"), "no ?");
        await page.waitForTimeout(3500);

        await caption(page, "You both talk to her at once, each in a conversation of your own");
        await Promise.all([talkTo(page, "Mara"), talkTo(other, "Mara")]);
        await Promise.all([readToChoices(page, 1000), readToChoices(other, 1000)]);
        await caption(page, "What she offers comes from each player's own quest log", { at: "top" });
        await page.waitForTimeout(4000);
        await Promise.all([leave(page), leave(other)]);

        await caption(page, "Pell runs a dice table");
        await standBelow(other, "Pell", 2);
        await talkTo(page, "Pell");
        await readToChoices(page, 900);
        await loseAtDice(page);
        await caption(page, "Losing again? This row warns what it does before you pick it", { at: "top" });
        await hoverUi(page, "You're cheating.");
        await page.waitForTimeout(2500);
        await pick(page, "You're cheating.");
        await conversationOver(page);
        await waitFor(
          page,
          ({ notifications }) => notifications.bubbles.some(({ npc }) => npc === "Pell"),
          "Pell never shouted",
        );
        await caption(page, `Pell shouts for the guards over his head — for you alone. ${name} sees none of them`);
        await fight(page, "HarbourGuard");
        await page.waitForTimeout(1500);

        await caption(page, "Pell holds a grudge now, and settles it himself");
        await talkTo(page, "Pell");
        await onStage(page, "PellGrudging");
        await readToChoices(page, 1000);
        await hoverUi(page, "Then let's settle this.");
        await page.waitForTimeout(2000);
        await pick(page, "Then let's settle this.");
        await conversationOver(page);
        await caption(page, `His table is empty for you while his copy fights you. On ${name}'s screen, Pell never left`);
        await page.waitForTimeout(3000);
        await fight(page, "PellHostile");
        await caption(page, `Pell is dead in your world for half an hour, and alive in ${name}'s`);
        await page.waitForTimeout(5000);
      } finally {
        await stop();
      }
    },
  }),
);

async function standBelow(page: Page, npc: string, dx: number): Promise<void> {
  await travelTo(page, (snapshot) => {
    const at = townsperson(snapshot, npc)?.at;
    const open = snapshot.walkable.filter((tile) => !occupied(snapshot, tile));
    return at && closestTile(open, [at[0] + dx, at[1] + 2.5]);
  });
  await waitUntilStill(page);
}

async function leave(page: Page): Promise<void> {
  await focusGame(page);
  await page.keyboard.press("Escape");
  await conversationOver(page);
}

async function loseAtDice(page: Page): Promise<void> {
  for (let roll = 0; roll < 20; roll++) {
    const before = await probe(page);
    if (before.stage?.node === "PellWins") return;
    const gold = holding(before, "Gold");
    await pick(page, before.stage?.node === "PellHello" ? "Roll the dice." : "Roll again.");
    const landed = await waitFor(
      page,
      (snapshot) => holding(snapshot, "Gold") !== gold && snapshot.stage && !snapshot.stage.typing && snapshot.stage,
      "the dice never landed",
    );
    if (landed.node === "PellLoses") {
      await caption(page, "A win pays twenty. Again!", { at: "top" });
    }
    await page.waitForTimeout(1500);
  }
  throw new Error("Pell never won a roll");
}

async function fight(page: Page, npc: string): Promise<void> {
  const foe = ({ me, actors }: Snapshot) =>
    me ? nearest(actors.filter((actor) => actor.npc === npc && actor.health > 0), me.at)?.aim : undefined;
  const deadline = Date.now() + 120_000;
  while (foe(await probe(page))) {
    if (Date.now() > deadline) throw new Error(`the ${npc} never fell`);
    await travelTo(page, foe).catch(() => undefined);
    await page.waitForTimeout(1500);
  }
}
