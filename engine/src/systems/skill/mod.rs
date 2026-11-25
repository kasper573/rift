pub mod aim;
pub mod bar;
pub mod card;
pub mod casting;
pub mod client;
pub mod hotbar;
pub mod learn;
pub mod prompt;
pub mod query;
pub mod reset;
pub mod window;

pub use casting::{CastAim, CastWay, UseSkill, UseSkillRequest, cast};
pub use hotbar::{AssignBasicAttackRequest, AssignHotbarRequest, BasicAttackPick};
pub use learn::LearnSkillsRequest;
pub use reset::{ResetAnswerRequest, ResetPrompt, ResetSkills};

use crate::core::script::ReachDef;
use crate::core::script::RunnerSpec;
use crate::core::script::Vocabulary;
use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::core::audio::playback::SfxId;
use crate::core::card::CardStyleId;
use crate::core::content::ImageRef;
use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Loading, ModuleRule, ModuleSet, NamedContent,
    Nearby, RowCheck, serial,
};
use crate::core::geometry::{PlacedShape, Shape, facing_from};
use crate::core::math::Pos;
use crate::core::script::FlowCx;
use crate::core::script::{
    EventKind, Expr, OwnerRow, RunnerKind, ScriptModule, ScriptedRow, Scripts,
};
use crate::core::text::LineText;
use crate::core::time::UnixMillis;
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::area::AreaTag;
use crate::systems::class::{self, NodeKey, SkillPoints, SkillTree};
use crate::systems::combat::Regard;
use crate::systems::crafting::{self, RecipeKey};
use crate::systems::equipment;
use crate::systems::item::{Inventory, ItemId, ItemKind, ItemStack};
use crate::systems::item::{USED_POINT, USED_RANK, USED_TARGET, Used};
use crate::systems::mention::Mention;
use crate::systems::messages::{self, messages};
use crate::systems::movement;
use crate::systems::npc::{Npc, NpcDef};
use crate::systems::roster::{Among, TickRoster};
use crate::systems::stat::{self, StatKind};
use crate::systems::status::aspect::{Activated, Deactivated, Interval};

messages! {
    /// What the game says of skills.
    pub struct SkillMessages {
        /// What a player without a class is told learning a skill.
        learn_without_class,
        /// What a player is told putting something on a bar slot there is none of.
        no_such_slot,
        /// What a player without a skill bar is told putting something on it.
        no_hotbar,
        /// What a player without a basic attack slot is told setting their basic attack.
        no_basic_attack_slot,
        /// What a player is told aiming a skill at a spot farther off than it reaches.
        spot_too_far,
        /// What a player is told casting at someone out of their sight.
        cant_see,
        /// What a player is told casting a skill, `{skill}`, at someone it can't land on.
        cant_target[skill: Line],
        /// What a player is told casting a skill, `{skill}`, that wants a target without one.
        needs_target[skill: Line],
        /// What a player is told casting an area skill, `{skill}`, without a point to aim at.
        needs_point[skill: Line],
        /// What a player is told casting a spot skill, `{skill}`, without a spot to aim at.
        needs_spot[skill: Line],
        /// What a player is told casting a skill, `{skill}`, that is still cooling down.
        cooling_down[skill: Line],
        /// What a player is told casting a skill, `{skill}`, while lacking something it needs, `{need}`.
        needs_unmet[need: Line, skill: Line],
        /// What a player is told casting a skill, `{skill}`, with too little mana, `{mana}` being what it takes.
        needs_mana[mana: Number, skill: Line],
        /// What a player is told using or placing a skill, `{skill}`, they have not learned.
        unknown_skill[skill: Line],
        /// What a player is told casting a passive skill, `{skill}`.
        passive_holds[skill: Line],
        /// What a player is told placing a passive skill, `{skill}`, on the bar.
        passive_unslotted[skill: Line],
        /// What a player is told placing an item, `{item}`, they don't have on the bar.
        item_not_held[item: Line],
        /// What a player is told picking a skill, `{skill}`, that can't be a basic attack.
        not_basic_attack[skill: Line],
        /// What a player is told picking an item, `{item}`, that grants no basic attack.
        item_no_basic_attack[item: Line],
        /// The notice that a skill, `{skill}`, has become the basic attack.
        basic_attack_set[skill: Line],
        /// What a player is told planning a node, `{node}`, their tree doesn't have.
        not_in_tree(node: Text),
        /// What a player is told planning a rank of a skill, `{skill}`, they already hold at `{rank}` or above.
        already_ranked[rank: Number, skill: Line],
        /// What a player is told planning a rank, `{rank}`, past what a skill, `{skill}`, has, `{count}` being how many ranks it has.
        rank_out_of_range[count: Number, rank: Number, skill: Line],
        /// What a player is told learning more than they can pay for, `{cost}` being the points it takes and `{held}` those they can spend.
        short_of_points(cost: Number, held: Number),
        /// The notice that a skill, `{skill}`, was learned.
        learned_skill[skill: Line],
        /// The notice that a skill, `{skill}`, reached a higher rank, `{rank}`.
        learned_rank[rank: Number, skill: Line],
        /// The notice that every skill was forgotten and the points returned.
        skills_reset,
        /// What the reset confirm dialog says is given up, `{ranks}` across `{skills}`, and given back, `{points}`.
        reset_summary(points: Number, ranks: Number, skills: Number),
        /// The title of the dialog that asks whether to reset skills.
        reset_title,
        /// What the reset confirm dialog says of the bar slots.
        reset_body,
        /// The button that confirms resetting skills.
        reset_confirm,
        /// The button that declines a skill or bar change in a confirm dialog.
        keep,
        /// Why a conversation ends when its player is struck.
        under_attack,
        /// What a player is told attacking with an empty basic attack slot.
        nothing_to_attack_with,
        /// What a player is told quick casting a skill, `{skill}`, with nothing under the pointer.
        nothing_to_aim_at[skill: Line],
        /// The prompt shown while aiming where to step off a mount.
        pick_shore,
        /// What the click does while aiming where a rider steps off a mount.
        step_off,
        /// What the dismiss key does while aiming where to step off a mount.
        stay_aboard,
        /// The prompt shown while aiming where to launch a mount on water.
        pick_water,
        /// What the click does while aiming where to launch a mount.
        launch,
        /// What the dismiss key does while aiming a skill at a spot.
        put_away,
        /// The prompt shown while aiming a skill at a spot.
        pick_spot,
        /// What the click does while aiming a skill at a spot.
        use_there,
        /// What marks a good spot to call a mount, `{mount}`, at.
        mount_here(mount: Text),
        /// What marks a good spot to step off at.
        step_off_here,
        /// The screen-reader name and tooltip of the grip that resizes the bar.
        reshape_bar,
        /// What the notice says while a bar slot, `{slot}`, waits for its new key, `{key}` being the key that cancels.
        rebind_prompt[key: Line, slot: Number],
        /// What a player is told pinning to a full bar.
        no_free_slot,
        /// The title of the dialog asking whether to take an entry, `{entry}`, off the bar.
        take_off_title[entry: Line],
        /// What the take-off dialog says of what remains.
        take_off_body,
        /// The button that takes an entry off the bar.
        take_off,
        /// What names an empty bar entry.
        nothing,
        /// What names an empty bar slot.
        empty,
        /// The screen-reader name of a bar slot, `{slot}` being its number and `{name}` what it holds.
        bar_slot_named(name: Text, slot: Number),
        /// What a bar slot shows while it waits for a new key.
        key_pending,
        /// The menu entry that rebinds a bar slot's key.
        rebind_key,
        /// The menu entry that empties a bar slot.
        clear_slot,
        /// What a bar slot's menu says of what it holds, `{name}`, and its key, `{key}`.
        slot_keyed[key: Line, name: Text],
        /// What a bar slot's menu says of what it holds, `{name}`, when it has no key.
        slot_unkeyed(name: Text),
        /// The screen-reader name of the basic attack slot, `{name}` being what it holds.
        basic_slot_named(name: Text),
        /// The menu entry that empties the basic attack slot.
        clear,
        /// What a bar slot shows over a skill that cools down for `{count}` more minutes.
        cooling_minutes(count: Number),
        /// What a bar slot shows over a skill that cools down for `{count}` more seconds.
        cooling_seconds(count: Number),
        /// The hint under the basic attack slot's skill card, with the keys `{interact}`, `{carry}`, `{pin}` and `{options}`.
        basic_attack_hint[carry: Line, interact: Line, options: Line, pin: Line],
        /// The hint on the empty basic attack slot, `{carry}` being the key that carries.
        basic_slot_empty_hint[carry: Line],
        /// The tooltip title of the empty basic attack slot.
        basic_slot_empty,
        /// The hint on a bar slot with a key, `{key}` being the key for its options.
        slot_options_hint[key: Line],
        /// What a bar slot's hint says when it has no key.
        slot_no_key,
        /// The hint that follows no key, `{key}` being the key for options.
        slot_bind_hint[key: Line],
        /// The hint on a bar slot that has `{key}` bound, `{usage}` being the slot input.
        slot_use_hint[key: Line, usage: Line],
        /// The hint on a bar slot, `{key}` being the key that empties it.
        slot_take_off_hint[key: Line],
        /// What the tooltip of an empty bar slot says first.
        slot_empty_hint,
        /// What the tooltip of an empty bar slot says of filling it, `{key}` being the key that carries.
        slot_fill_hint[key: Line],
        /// The tooltip title of an empty bar slot, `{slot}` being its number.
        slot_titled(slot: Number),
        /// What a player is told using an item, `{item}`, they have none of.
        item_used_up[item: Line],
        /// What a player is told binding a key, `{key}`, the game already uses for `{action}`.
        key_taken[action: Text, key: Line],
        /// What a player is told when a key, `{key}`, moves from bar slot `{lost}` to slot `{gained}`.
        key_moved[gained: Number, key: Line, lost: Number],
        /// The mark on an item's card for the skill it grants as a basic attack.
        basic_attack_mark,
        /// The band of an active skill's card.
        active_band,
        /// The band of a passive skill's card.
        passive_band,
        /// The heading of a skill card's ranks.
        heading_ranks,
        /// The heading of what a skill card's masteries make.
        heading_masters,
        /// The heading of what unlocked a learned skill.
        heading_unlocked_by,
        /// The heading of what a skill not yet learned costs.
        heading_costs,
        /// The heading of what a skill needs to be used.
        heading_to_use,
        /// What a skill card says when using the skill needs nothing.
        nothing_needed,
        /// What a skill's tooltip says of a skill not learned.
        not_learned,
        /// What a skill's tooltip says of a skill not learned that still needs `{need}`.
        not_learned_needs[need: Line],
        /// What a passive skill's tooltip says, `{tally}` being its rank and `{usage}` how it works.
        tooltip_passive(count: Number, rank: Number, usage: Text),
        /// What an active skill's tooltip says, `{tally}` being its rank and `{usage}` how it works.
        tooltip_active(count: Number, rank: Number, usage: Text),
        /// A skill's rank, `{rank}` of `{count}`.
        rank_tally(count: Number, rank: Number),
        /// What a skill's card says of a skill lent by a worn item, `{item}`.
        lent_line[item: Line],
        /// What a passive skill's card says of how it works.
        holds_while_known,
        /// How a skill that lands on one ally is described.
        target_single_ally,
        /// How a skill that lands on one enemy is described.
        target_single_enemy,
        /// How a skill that lands on allies in an area is described.
        target_area_ally,
        /// How a skill that lands on enemies in an area is described.
        target_area_enemy,
        /// How a skill that lands on allies in an area from where its caster stands is described.
        target_melee_ally,
        /// How a skill that lands on enemies in an area from where its caster stands is described.
        target_melee_enemy,
        /// How a skill aimed at a spot is described.
        target_spot,
        /// How a skill that lands on its caster is described.
        target_caster,
        /// How a circular area is described, `{radius}` being its radius in meters.
        area_circle(radius: Number),
        /// How a rectangular area is described, `{width}` by `{height}` meters.
        area_rect(height: Number, width: Number),
        /// How a cone is described, `{angle}` degrees wide and `{length}` meters long.
        area_cone(angle: Number, length: Number),
        /// How a line is described, `{length}` meters long and `{width}` wide.
        area_line(length: Number, width: Number),
        /// How far a skill reaches on its card, `{meters}` in meters.
        range_of(meters: Number),
        /// How a skill's cooldown, `{seconds}`, is described.
        cooldown_of(seconds: Number),
        /// How a rank is introduced on a skill card, `{rank}` being the rank and `{cost}` its cost in points.
        rank_cost(cost: Number, rank: Number),
        /// How a rank is introduced on a skill card when it changes stats: `{rank}`, its cost in points `{cost}` and `{changes}`, the list of the stats it changes.
        rank_cost_changing(rank: Number, cost: Number, changes: Text),
        /// How a skill card says a rank changes a stat: `{stat}` is `power`, `cooldown`, `range`, `radius`, `duration` or `mana`, and `{value}` the change, up or down as `{direction}` says.
        stat_change(stat: Text, value: Number, direction: Text),
        /// What a skill card says it spends, `{mana}` being the mana.
        spends_mana(mana: Number),
        /// What a skill card says it spends, `{item}`, with `{held}` how many the player holds.
        spends_item[held: Number, item: Line],
        /// The hint of the key that raises a rank in the skill window.
        raise,
        /// The hint of the key that lowers a planned rank in the skill window.
        lower,
        /// The hint of the key that carries a skill to the bar.
        to_the_bar,
        /// The hint of the key that opens a skill's card.
        open_the_card,
        /// What the skill window says when the player has no class.
        no_class,
        /// How a locked node of the skill tree is named to a screen reader.
        state_locked,
        /// How an available node of the skill tree is named to a screen reader.
        state_available,
        /// How a learned node of the skill tree is named to a screen reader.
        state_learned,
        /// How a node of the skill tree at its last rank is named to a screen reader.
        state_maxed,
        /// How a planned node of the skill tree is named to a screen reader, `{rank}` being the planned rank.
        state_picked(rank: Number),
        /// The screen-reader name of a node of the skill tree, `{skill}` being the skill and `{state}` its state.
        node_named(skill: Text, state: Text),
        /// The screen-reader name of the skill tree.
        skill_tree,
        /// What the skill window says of the points left, `{points}`.
        points_to_spend(points: Number),
        /// What the skill window says of the points planned, `{points}`.
        points_picked(points: Number),
        /// The button that learns the planned ranks.
        learn,
        /// What the learn button's tooltip says.
        learn_tooltip,
        /// The button that drops the planned ranks.
        reset_plan,
        /// What the reset plan button's tooltip says.
        reset_plan_tooltip,
        /// The hint of the key that plans a rank on a skill's tooltip.
        hint_picks_rank,
        /// The hint of the key that takes a planned rank back on a skill's tooltip.
        hint_takes_back,
        /// The hint of the key that shows a skill's card.
        hint_shows_card,
        /// The hint of the key that puts a skill on the bar.
        hint_puts_on_bar,
        /// The hint of the key that carries a skill to a slot.
        hint_carries,
        /// The hint of the key that adds a skill to chat.
        hint_adds_to_chat,
    }
}

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;

    use bevy_replication::AppReplicationExt;
    app.replicate_private::<Learned>()
        .replicate_private::<Cooldowns>()
        .replicate_private::<Hotbar>()
        .replicate_private::<BasicAttack>()
        .replicate_private::<ResetPrompt>()
        .add_mapped_client_message::<UseSkillRequest>()
        .add_client_message::<LearnSkillsRequest>()
        .add_client_message::<AssignHotbarRequest>()
        .add_client_message::<AssignBasicAttackRequest>()
        .add_client_message::<ResetAnswerRequest>();
}

/// Every request a client may send about its skills, answered in one place.
pub fn requests(world: &mut World) {
    casting::request(world);
    learn::requests(world);
    hotbar::requests(world);
    reset::answers(world);
}

pub type SkillId = ContentId<SkillDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct SkillDef {
    pub name: String,
    pub icon: ImageRef,
    #[serde(default, skip_serializing_if = "SkillKind::is_active")]
    pub kind: SkillKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfx: Option<SfxId>,
    pub target: Targeting,
    #[serde(default, skip_serializing_if = "Seconds::is_zero")]
    pub cooldown: Seconds,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Expr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cost: Vec<ItemStack>,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub basic_attack: bool,
    pub levels: Vec<SkillLevel>,
    /// The recipes whoever knows the skill masters, for as long as they know it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub masteries: Vec<RecipeKey>,
    /// What the skill's card is drawn in. Left out, the game's default card style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_style: Option<CardStyleId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<SkillModule>,
}

impl ContentFile for SkillDef {
    const TABLE: &'static str = "skill";

    fn fresh(near: &Nearby) -> Result<SkillDef, String> {
        let first = SkillLevel {
            cost: SkillPoints(1),
            stats: BTreeMap::new(),
        };
        Ok(SkillDef {
            name: near.name(),
            icon: near.image()?,
            kind: SkillKind::Active,
            sfx: None,
            target: Targeting::Single {
                affects: Affects::Enemy,
                range: None,
            },
            cooldown: Seconds(0.0),
            requires: None,
            cost: Vec::new(),
            basic_attack: false,
            levels: vec![first],
            masteries: Vec::new(),
            card_style: None,
            modules: Vec::new(),
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 40);
        check.count("/levels", self.levels.len(), 1, 20, "ranks");
        if self.kind == SkillKind::Passive {
            let passive = "nothing: a passive skill is never used, it holds while known";
            if self.target != Targeting::Caster {
                check.error(
                    "/target",
                    "passive with a target",
                    self.target.label(),
                    "self",
                );
            }
            if self.cooldown.0 > 0.0 {
                check.error(
                    "/cooldown",
                    "passive with a cooldown",
                    self.cooldown.0,
                    passive,
                );
            }
            if !self.cost.is_empty() {
                check.error("/cost", "passive with a cost", "a cost", passive);
            }
            if self.takes_mana() {
                check.error("/levels", "passive that takes mana", "mana", passive);
            }
            if self.basic_attack {
                check.error(
                    "/basic_attack",
                    "passive basic attack",
                    "true",
                    "false: a basic attack is used on a click",
                );
            }
        }
        if self.basic_attack {
            let answers = "nothing: a click on an enemy must always answer";
            if !self.cost.is_empty() {
                check.error("/cost", "basic attack with a cost", "a cost", answers);
            }
            if self.takes_mana() {
                check.error("/levels", "basic attack that takes mana", "mana", answers);
            }
            if self.cooldown.0 > 0.0 {
                check.error(
                    "/cooldown",
                    "basic attack with a cooldown",
                    self.cooldown.0,
                    answers,
                );
            }
        }
        if self.cooldown.0 < 0.0 {
            check.error("/cooldown", "out of range", self.cooldown.0, "0 or more");
        }
        if let Some(Some(range)) = self.target.range()
            && range.0 <= 0.0
        {
            check.error("/target/range", "out of range", range.0, "more than 0 m");
        }
        for (index, level) in self.levels.iter().enumerate() {
            let unused = |stat: &SkillStat| match stat {
                SkillStat::Range => self.target.range().is_none(),
                SkillStat::Radius => self.target.shape().is_none(),
                SkillStat::Power | SkillStat::Cooldown | SkillStat::Duration | SkillStat::Mana => {
                    false
                }
            };
            for stat in level.stats.keys().filter(|stat| unused(stat)) {
                check.error(
                    format!("/levels/{index}/stats/{}", stat.key()),
                    "stat the targeting doesn't use",
                    self.target.label(),
                    "a targeting with a range for range, or with a shape for radius",
                );
            }
        }
        for (index, stack) in self.cost.iter().enumerate() {
            if stack.count == 0 {
                check.error(
                    format!("/cost/{index}/count"),
                    "empty stack",
                    0,
                    "at least 1",
                );
            }
        }
    }
}

impl NamedContent for SkillDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ModuleSet for SkillDef {
    type Module = SkillModule;

    fn modules(&self) -> &[SkillModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<SkillModule> {
        &mut self.modules
    }

    fn rule(_: &SkillModule) -> ModuleRule {
        ModuleRule::AnyNumber
    }
}

impl ScriptedRow for SkillDef {
    const OWNER: RunnerKind = SKILL_RUNNER;

    fn script(module: &SkillModule) -> Option<&ScriptModule> {
        match module {
            SkillModule::Script(script) => Some(script),
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillModule {
    Script(ScriptModule),
}

/// An active skill is used, aimed and performed; a passive one holds while it is known, its
/// script running as an aspect of the character.
#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "title_case")]
pub enum SkillKind {
    #[default]
    Active,
    Passive,
}

impl SkillKind {
    pub fn is_active(&self) -> bool {
        *self == SkillKind::Active
    }

    pub fn label(self) -> &'static str {
        self.into()
    }
}

/// What a skill is aimed at, and so whether its caster walks before using it, with only what
/// that kind of aim uses.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, IntoStaticStr)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[strum(serialize_all = "snake_case")]
pub enum Targeting {
    /// The casting actor themselves.
    Caster,
    /// A single actor it affects, walked into range of.
    Single {
        affects: Affects,
        /// Left out, the caster's Range stat.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("default" = 4.0))]
        range: Option<Meters>,
    },
    /// A shape centred on a point, walked into range of.
    RangedArea {
        affects: Affects,
        /// Left out, the caster's Range stat.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("default" = 4.0))]
        range: Option<Meters>,
        shape: Shape,
        /// How many of those inside the shape a cast lands on, the nearest its centre first;
        /// without a cap, on every one of them.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_targets: Option<u32>,
    },
    /// A shape from where the caster stands, toward the point aimed at, which is never walked
    /// toward: however far off, it only says which way the caster turns.
    MeleeArea {
        affects: Affects,
        shape: Shape,
        /// How many of those inside the shape a cast lands on, the nearest the caster first;
        /// without a cap, on every one of them.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_targets: Option<u32>,
    },
    /// A spot within range of where the caster stands, which they are never walked toward:
    /// nothing is hit there, and what comes of it is the skill's script's to say.
    Spot {
        /// Left out, the caster's Range stat.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("default" = 4.0))]
        range: Option<Meters>,
    },
}

/// Whom a skill lands on, by how its caster regards them: the part of `Regard` a skill may name.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Affects {
    /// The caster's allies, and the caster.
    Ally,
    Enemy,
}

impl From<Affects> for Regard {
    fn from(affects: Affects) -> Regard {
        match affects {
            Affects::Ally => Regard::Ally,
            Affects::Enemy => Regard::Enemy,
        }
    }
}

impl Targeting {
    pub fn affects(self) -> Option<Affects> {
        match self {
            Targeting::Single { affects, .. }
            | Targeting::RangedArea { affects, .. }
            | Targeting::MeleeArea { affects, .. } => Some(affects),
            Targeting::Caster | Targeting::Spot { .. } => None,
        }
    }

    /// How far from the caster it may be aimed, for the kinds aimed within a range: as the skill
    /// sets it, or left to the caster's Range stat.
    pub fn range(self) -> Option<Option<Meters>> {
        match self {
            Targeting::Single { range, .. }
            | Targeting::RangedArea { range, .. }
            | Targeting::Spot { range } => Some(range),
            Targeting::Caster | Targeting::MeleeArea { .. } => None,
        }
    }

    /// Whether the caster walks until what it is aimed at is in range.
    pub fn walks(self) -> bool {
        matches!(
            self,
            Targeting::Single { .. } | Targeting::RangedArea { .. }
        )
    }

    pub fn shape(self) -> Option<Shape> {
        match self {
            Targeting::RangedArea { shape, .. } | Targeting::MeleeArea { shape, .. } => Some(shape),
            Targeting::Caster | Targeting::Single { .. } | Targeting::Spot { .. } => None,
        }
    }

    pub fn max_targets(self) -> Option<u32> {
        match self {
            Targeting::RangedArea { max_targets, .. }
            | Targeting::MeleeArea { max_targets, .. } => max_targets,
            Targeting::Caster | Targeting::Single { .. } | Targeting::Spot { .. } => None,
        }
    }

    pub fn aimed_at_point(self) -> bool {
        matches!(
            self,
            Targeting::RangedArea { .. } | Targeting::MeleeArea { .. } | Targeting::Spot { .. }
        )
    }

    pub fn hostile(self) -> bool {
        self.affects() == Some(Affects::Enemy)
    }

    /// Where a cast toward `point` by a caster standing at `from` lays the shape down.
    pub fn place(self, shape: Shape, from: Pos<Meters>, point: Pos<Meters>) -> PlacedShape {
        let facing = facing_from(from, point);
        match self {
            Targeting::MeleeArea { .. } => PlacedShape::from_start(shape, from, facing),
            _ => PlacedShape::centred(shape, point, facing),
        }
    }

    /// What those a cast lands on are counted nearest to, for a cast toward `point` by a caster
    /// standing at `from`.
    pub fn anchor(self, from: Pos<Meters>, point: Pos<Meters>) -> Pos<Meters> {
        match self {
            Targeting::MeleeArea { .. } => from,
            _ => point,
        }
    }

    pub fn label(self) -> &'static str {
        self.into()
    }

    pub fn named(self, words: &SkillMessages) -> &str {
        match self {
            Targeting::Single {
                affects: Affects::Ally,
                ..
            } => words.target_single_ally(),
            Targeting::Single {
                affects: Affects::Enemy,
                ..
            } => words.target_single_enemy(),
            Targeting::RangedArea {
                affects: Affects::Ally,
                ..
            } => words.target_area_ally(),
            Targeting::RangedArea {
                affects: Affects::Enemy,
                ..
            } => words.target_area_enemy(),
            Targeting::MeleeArea {
                affects: Affects::Ally,
                ..
            } => words.target_melee_ally(),
            Targeting::MeleeArea {
                affects: Affects::Enemy,
                ..
            } => words.target_melee_enemy(),
            Targeting::Spot { .. } => words.target_spot(),
            Targeting::Caster => words.target_caster(),
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillLevel {
    pub cost: SkillPoints,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stats: BTreeMap<SkillStat, f32>,
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "title_case")]
pub enum SkillStat {
    Power,
    Cooldown,
    Range,
    Radius,
    Duration,
    /// How much mana a use takes.
    Mana,
}

impl SkillStat {
    pub fn key(self) -> &'static str {
        match self {
            SkillStat::Power => "power",
            SkillStat::Cooldown => "cooldown",
            SkillStat::Range => "range",
            SkillStat::Radius => "radius",
            SkillStat::Duration => "duration",
            SkillStat::Mana => "mana",
        }
    }
}

/// How far into a skill's ranks someone is: `NONE` outside them, `FIRST` at the first.
#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
)]
#[serde(transparent)]
pub struct Rank(pub u8);

impl Rank {
    pub const NONE: Rank = Rank(0);
    pub const FIRST: Rank = Rank(1);

    pub fn first() -> Rank {
        Rank::FIRST
    }

    pub fn is_first(&self) -> bool {
        *self == Rank::FIRST
    }
}

/// The ranks a character learned in its class tree, by node.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Learned(pub BTreeMap<NodeKey, Rank>);

impl Learned {
    pub fn rank<K: Ord + ?Sized>(&self, node: &K) -> Rank
    where
        NodeKey: Borrow<K>,
    {
        self.0.get(node).copied().unwrap_or(Rank::NONE)
    }

    /// The skills the learned nodes stand for, at their ranks.
    pub fn skills<'a>(&'a self, tree: &'a SkillTree) -> impl Iterator<Item = (SkillId, Rank)> + 'a {
        self.0.iter().filter_map(|(key, rank)| {
            let node = tree.node(key)?;
            (*rank > Rank::NONE).then_some((node.skill, *rank))
        })
    }

    pub fn cost(&self, content: &Content, tree: &SkillTree) -> SkillPoints {
        self.skills(tree)
            .fold(SkillPoints(0), |sum, (skill, rank)| {
                sum + skill.get(content).cost_to(rank)
            })
    }

    pub fn ranks(&self) -> u32 {
        self.0.values().map(|rank| u32::from(rank.0)).sum()
    }

    pub fn skills_held(&self) -> u32 {
        self.0.values().filter(|rank| **rank > Rank::NONE).count() as u32
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Cooldowns(pub BTreeMap<SkillId, UnixMillis>);

impl Cooldowns {
    pub fn ready_at(&self, skill: SkillId) -> Option<UnixMillis> {
        self.0.get(&skill).copied()
    }

    pub fn cooling(&self, skill: SkillId, now: UnixMillis) -> bool {
        self.ready_at(skill).is_some_and(|ready| ready > now)
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Hotbar(pub Vec<HotbarEntry>);

/// A place on the bar, counted from zero; the player sees it numbered from one.
#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct HotbarSlot(pub u32);

impl HotbarSlot {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub fn number(self) -> u32 {
        self.0 + 1
    }

    /// The first `count` slots, in order.
    pub fn first(count: u32) -> impl Iterator<Item = HotbarSlot> {
        (0..count).map(HotbarSlot)
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HotbarEntry {
    #[default]
    Empty,
    Skill(SkillId),
    Item(ItemId),
}

impl Hotbar {
    pub fn entry(&self, slot: HotbarSlot) -> HotbarEntry {
        self.0.get(slot.index()).copied().unwrap_or_default()
    }

    /// Puts `entry` in `slot`, taking it out of any other slot: the bar holds each skill or item
    /// once, so slotting it again moves it.
    pub fn set(&mut self, slot: HotbarSlot, entry: HotbarEntry) {
        if entry != HotbarEntry::Empty {
            for held in &mut self.0 {
                if *held == entry {
                    *held = HotbarEntry::Empty;
                }
            }
        }
        if self.0.len() <= slot.index() {
            self.0.resize(slot.index() + 1, HotbarEntry::Empty);
        }
        self.0[slot.index()] = entry;
        while self.0.last() == Some(&HotbarEntry::Empty) {
            self.0.pop();
        }
    }

    pub fn first_free(&self, within: u32) -> Option<HotbarSlot> {
        HotbarSlot::first(within).find(|slot| self.entry(*slot) == HotbarEntry::Empty)
    }

    pub fn slot_of(&self, entry: HotbarEntry) -> Option<HotbarSlot> {
        self.0
            .iter()
            .position(|held| *held == entry)
            .map(|slot| HotbarSlot(slot as u32))
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BasicAttack(pub Option<SkillId>);

impl SkillDef {
    pub fn rank_count(&self) -> u8 {
        self.levels.len() as u8
    }

    /// The ranks someone may hold the skill at.
    pub fn ranks(&self) -> RangeInclusive<Rank> {
        Rank::FIRST..=Rank(self.rank_count())
    }

    pub fn cost_to(&self, rank: Rank) -> SkillPoints {
        self.levels
            .iter()
            .take(usize::from(rank.0))
            .fold(SkillPoints(0), |sum, level| sum + level.cost)
    }

    /// A stat summed over the ranks reached.
    pub fn stat(&self, rank: Rank, stat: SkillStat) -> f32 {
        self.levels
            .iter()
            .take(usize::from(rank.0))
            .filter_map(|level| level.stats.get(&stat))
            .sum()
    }

    pub fn cooldown_at(&self, rank: Rank) -> Seconds {
        Seconds((self.cooldown.0 + self.stat(rank, SkillStat::Cooldown)).max(0.0))
    }

    /// How much mana a use at `rank` takes.
    pub fn mana_at(&self, rank: Rank) -> f32 {
        self.stat(rank, SkillStat::Mana).max(0.0)
    }

    fn takes_mana(&self) -> bool {
        self.levels
            .iter()
            .any(|level| level.stats.contains_key(&SkillStat::Mana))
    }

    pub fn shape_at(&self, rank: Rank) -> Option<Shape> {
        self.target
            .shape()
            .map(|shape| shape.grown(Meters(self.stat(rank, SkillStat::Radius))))
    }
}

/// The one complaint about a row holding a skill at a rank outside its ranks.
pub fn check_rank(
    check: &mut RowCheck,
    pointer: String,
    content: &Content,
    skill: SkillId,
    rank: Rank,
) {
    let def = skill.get(content);
    if !def.ranks().contains(&rank) {
        check.error(
            pointer,
            "rank out of range",
            rank.0,
            format!(
                "1 to {}, the ranks of {}",
                def.rank_count(),
                skill.path(content)
            ),
        );
    }
}

/// Every skill an actor may use now, at the rank it has it: an npc's listed skills, the ranks
/// learned in its class tree, and what its worn equipment, the usable items it carries or an
/// npc's basic attack grant at rank one.
pub fn known(world: &World, actor: Entity) -> BTreeMap<SkillId, Rank> {
    let content = world.resource::<Content>();
    let tree = class::of(world, actor).map(|class| &class.get(content).tree);
    let worn = equipment::worn(world, actor);
    let carried = world
        .get::<Inventory>(actor)
        .into_iter()
        .flat_map(|bag| bag.stacks().map(|(_, stack)| stack.item));
    known_from(
        content,
        tree,
        world.get::<Learned>(actor),
        worn,
        carried,
        world.get::<Npc>(actor).map(|npc| npc.def.get(content)),
    )
}

/// `known`, from the parts a character is made of, so a snapshot is checked the way the world
/// answers for the character it becomes.
pub fn known_from(
    content: &Content,
    tree: Option<&SkillTree>,
    learned: Option<&Learned>,
    worn: impl Iterator<Item = ItemId>,
    carried: impl Iterator<Item = ItemId>,
    npc: Option<&NpcDef>,
) -> BTreeMap<SkillId, Rank> {
    let mut known = BTreeMap::new();
    if let (Some(tree), Some(learned)) = (tree, learned) {
        known.extend(learned.skills(tree));
    }
    for skill in worn.flat_map(|item| grants_of(content, item)) {
        known.entry(*skill).or_insert(Rank::FIRST);
    }
    for skill in carried.filter_map(|item| cast_by(content, item)) {
        known.entry(skill).or_insert(Rank::FIRST);
    }
    if let Some(npc) = npc {
        known.extend(npc.skills.iter().map(|listed| (listed.skill, listed.rank)));
        if let Some(skill) = npc.basic_attack {
            known.entry(skill).or_insert(Rank::FIRST);
        }
    }
    known
}

/// Why an actor could not use the skill at all, whatever its aim or readiness.
pub fn unusable(world: &World, actor: Entity, skill: SkillId) -> Option<LineText> {
    let content = world.resource::<Content>();
    if rank_of(world, actor, skill) == Rank::NONE {
        Some(unknown(skill, content))
    } else {
        passive(world, skill)
    }
}

/// Why `skill` is never cast: it is passive, and holds on its own.
pub fn passive(world: &World, skill: SkillId) -> Option<LineText> {
    let content = world.resource::<Content>();
    (skill.get(content).kind == SkillKind::Passive).then(|| {
        messages::of(content)
            .skill
            .passive_holds(Mention::Skill(skill).line(content))
    })
}

/// Why a passive skill takes no bar slot.
pub fn unslotted(skill: SkillId, content: &Content) -> LineText {
    messages::of(content)
        .skill
        .passive_unslotted(Mention::Skill(skill).line(content))
}

/// Why a skill the actor has not learned is turned down.
pub fn unknown(skill: SkillId, content: &Content) -> LineText {
    messages::of(content)
        .skill
        .unknown_skill(Mention::Skill(skill).span(content))
}

pub fn rank_of(world: &World, actor: Entity, skill: SkillId) -> Rank {
    known(world, actor)
        .get(&skill)
        .copied()
        .unwrap_or(Rank::NONE)
}

/// The skills an actor's worn equipment grants, in slot order.
pub fn granted(world: &World, actor: Entity) -> Vec<SkillId> {
    let content = world.resource::<Content>();
    equipment::worn(world, actor)
        .flat_map(|item| grants_of(content, item).iter().copied())
        .collect()
}

pub fn grants_of(content: &Content, item: ItemId) -> &[SkillId] {
    match &item.get(content).kind {
        ItemKind::Equipment { grants, .. } => grants,
        _ => &[],
    }
}

/// Whether using `skill` is using an item `actor` carries: one whose use casts it.
pub fn carried_as_item(world: &World, actor: Entity, skill: SkillId) -> bool {
    let content = world.resource::<Content>();
    world.get::<Inventory>(actor).is_some_and(|bag| {
        bag.stacks()
            .any(|(_, stack)| cast_by(content, stack.item) == Some(skill))
    })
}

/// The skill a use of the item casts, for a usable item that casts one.
pub fn cast_by(content: &Content, item: ItemId) -> Option<SkillId> {
    match item.get(content).kind {
        ItemKind::Usable { casts } => casts,
        _ => None,
    }
}

/// The first skill flagged a basic attack among what the worn items grant, in slot order.
pub fn first_flagged_grant(
    content: &Content,
    worn: impl Iterator<Item = ItemId>,
) -> Option<SkillId> {
    worn.flat_map(|item| grants_of(content, item).iter().copied())
        .find(|skill| skill.get(content).basic_attack)
}

/// The reach of a skill from an actor: the row's range, or the actor's Range stat when unset,
/// plus whatever its ranks add.
/// How far from where `actor` stands the skill may be aimed at `rank`; none for a skill used
/// where its caster stands.
pub fn range_of(world: &World, actor: Entity, skill: SkillId, rank: Rank) -> Option<Meters> {
    let def = skill.get(world.resource::<Content>());
    let base = def
        .target
        .range()?
        .unwrap_or_else(|| Meters(stat::effective(world, actor, StatKind::Range)));
    Some(Meters((base.0 + def.stat(rank, SkillStat::Range)).max(0.0)))
}

/// Whom a cast lands on, read from the use that raised the running script: the caster, the aimed
/// actor when it still stands where the skill may reach it, or the actors inside the shape, the
/// nearest its anchor first.
pub fn targets(world: &World, cx: &FlowCx, skill: SkillId) -> Vec<Entity> {
    let Some(caster) = cx.subject else {
        return Vec::new();
    };
    let target = skill.get(world.resource::<Content>()).target;
    match target {
        Targeting::Caster => vec![caster],
        Targeting::Spot { .. } => Vec::new(),
        Targeting::Single { .. } => used_target(world, cx)
            .filter(|who| casting::fits(world, caster, *who, target))
            .into_iter()
            .collect(),
        Targeting::RangedArea { max_targets, .. } | Targeting::MeleeArea { max_targets, .. } => {
            let Some((footprint, anchor)) = laid(world, cx, skill) else {
                return Vec::new();
            };
            let Some(roster) = world.get_resource::<TickRoster>() else {
                return Vec::new();
            };
            let Some(area) = world.get::<AreaTag>(caster).map(|tag| tag.area) else {
                return Vec::new();
            };
            let mut inside: Vec<(Entity, Meters)> = roster
                .near(Among::Anyone, footprint.at, footprint.reach())
                .filter(|listed| listed.area == area && footprint.contains(listed.at))
                .filter(|listed| casting::fits(world, caster, listed.entity, target))
                .map(|listed| (listed.entity, listed.at.distance(anchor)))
                .collect();
            inside.sort_by(|(a, da), (b, db)| da.0.total_cmp(&db.0).then(a.cmp(b)));
            inside
                .into_iter()
                .take(max_targets.map_or(usize::MAX, |most| most as usize))
                .map(|(entity, _)| entity)
                .collect()
        }
    }
}

/// The ground a cast covers, laid as its targeting lays it toward the aimed point.
pub fn footprint(world: &World, cx: &FlowCx, skill: SkillId) -> Option<PlacedShape> {
    laid(world, cx, skill).map(|(footprint, _)| footprint)
}

pub fn aimed_point(world: &World, cx: &FlowCx, skill: SkillId) -> Option<Pos<Meters>> {
    let caster = cx.subject?;
    match skill.get(world.resource::<Content>()).target {
        Targeting::Caster => movement::position(world, caster),
        Targeting::Single { .. } => movement::position(world, used_target(world, cx)?),
        Targeting::RangedArea { .. } | Targeting::MeleeArea { .. } | Targeting::Spot { .. } => {
            cx.event.get(USED_POINT)?.as_point()
        }
    }
}

/// The ground a cast covers, with the point those it lands on are counted nearest to.
fn laid(world: &World, cx: &FlowCx, skill: SkillId) -> Option<(PlacedShape, Pos<Meters>)> {
    let caster = cx.subject?;
    let def = skill.get(world.resource::<Content>());
    let shape = def.shape_at(used_rank(cx))?;
    let from = movement::position(world, caster)?;
    let point = aimed_point(world, cx, skill).unwrap_or(from);
    Some((
        def.target.place(shape, from, point),
        def.target.anchor(from, point),
    ))
}

fn used_target(world: &World, cx: &FlowCx) -> Option<Entity> {
    let target = cx.event.get(USED_TARGET)?.as_actor()?;
    world.get_entity(target).ok().map(|entity| entity.id())
}

fn used_rank(cx: &FlowCx) -> Rank {
    cx.event
        .get(USED_RANK)
        .and_then(|rank| u8::try_from(rank.as_int()).ok())
        .map_or(Rank::FIRST, Rank)
}

/// The rules a skill's script keeps: an active skill answers `used` and no aspect event, a passive
/// one the other way around.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let scripts = content.built::<Scripts>();
    for (id, def) in loading.rows::<SkillDef>(&content) {
        for (index, mastery) in def.masteries.iter().enumerate() {
            if let Some(problem) = crafting::mastery_problem(&content, mastery) {
                loading.row_error(id, &format!("/masteries/{index}"), problem);
            }
        }
        let may_do = scripts.may_do(OwnerRow::of_row(id));
        let aspect_events = [
            EventKind::of::<Activated>(),
            EventKind::of::<Deactivated>(),
            EventKind::of::<Interval>(),
        ];
        let (rule, expected) = match def.kind {
            SkillKind::Passive if may_do.answers(EventKind::of::<Used>()) => (
                "passive answers used",
                "activated, deactivated or interval: a passive skill is never used",
            ),
            SkillKind::Active if aspect_events.into_iter().any(|kind| may_do.answers(kind)) => (
                "active answers an aspect event",
                "used: an active skill's script runs when it is used",
            ),
            SkillKind::Active if !may_do.answers(EventKind::of::<Used>()) => (
                "skill without used",
                "a script answering used on every active skill",
            ),
            _ => continue,
        };
        loading.row_error(
            id,
            "/modules",
            ContentProblem::new(rule, format!("skill/{}", id.path(&content)), expected),
        );
    }
}

pub const SKILL_RUNNER: RunnerKind = RunnerKind::new("skill");

/// A script that may cast a skill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Casts(pub SkillId);

impl ReachDef for Casts {}

pub fn scripting(vocabulary: &mut Vocabulary) {
    query::scripting(vocabulary);
    vocabulary
        .runner(
            SKILL_RUNNER,
            RunnerSpec::named_row::<SkillDef>()
                .stateless()
                .starting_with_world(),
        )
        .binder(|loading, content, bind| bind.bind_table::<SkillDef>(loading, content));
    casting::scripting(vocabulary);
    reset::scripting(vocabulary);
}
