import { test } from "@playwright/test";

import { caption, chapter } from "../helpers/demo";
import { arriveSeasoned, enter, hunt, meet, walkAlong } from "../helpers/tour";

test(
  "Gloomhollow",
  chapter({
    summary: "A burial town the dead have overrun: the chapel and the crypt",
    setup: (page) => arriveSeasoned(page, "Gloomhollow", [2.5, 35]),
    play: async (page) => {
      await caption(page, "Gloomhollow: a burial town in a wooded hollow, overrun since its oldest graves opened");
      await walkAlong(page, [[8, 35]], 400);
      await caption(page, "Ghosts haunt the streets where the townsfolk fled");
      await hunt(page, "Ghost", "haunted-streets", 1);
      await caption(page, "The chapel still stands on the square; the living shelter inside");
      await enter(page, "chapel-door");
      await caption(page, "The nave, with its altar and pews; the gravedigger keeps to the back pew");
      await meet(page, "Gravedigger", 3000);
      await caption(page, "The priest's quarters lie through the dividing wall");
      await meet(page, "Priest", 3000);
      await enter(page, "nave-door");
      await caption(page, "In the churchyard, the crypt's keeper guards the shut vault");
      await enter(page, "crypt");
      await meet(page, "CryptKeeper", 3500);
      await enter(page, "crypt-gate");
    },
  }),
);
