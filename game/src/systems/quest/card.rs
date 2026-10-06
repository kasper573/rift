use std::hash::{DefaultHasher, Hash, Hasher};

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::tokens::{palette, spacing, typography};
use ui::{ChipOptions, Family, component};

use super::{
    ActiveQuest, Objective, Progress, QuestDef, QuestId, QuestLog, clock_label, resets_label,
};
use crate::core::assets::AssetRef;
use crate::systems::dialogue::{Conversation, stage};
use crate::systems::item::{self, Inventory, ItemStack};
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;

const WIDTH: f32 = 480.0;
const ICON: f32 = 20.0;
const PICK: f32 = 32.0;

pub struct QuestCardPlugin;

impl Plugin for QuestCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShownCard>()
            .add_systems(
                Update,
                (show_card, tick_clocks).run_if(in_state(GameScene::Area)),
            )
            .add_systems(OnExit(GameScene::Area), forget);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Moment {
    Offer,
    TurnIn,
    Decision,
}

#[derive(Resource, Default)]
struct ShownCard(Option<(QuestId, Moment)>);

#[derive(Component, Default, Clone)]
struct QuestCard;

#[derive(Component, Default, Clone)]
struct QuestCardBody;

fn forget(mut shown: ResMut<ShownCard>) {
    shown.0 = None;
}

fn show_card(world: &mut World) {
    let seen = world.resource::<Viewpoint>().0;
    let moment = seen
        .and_then(|seen| world.get::<Conversation>(seen))
        .and_then(|conversation| moment_of(conversation.node));
    if world.resource::<ShownCard>().0 != moment {
        world.resource_mut::<ShownCard>().0 = moment;
        let shown: Vec<Entity> = world
            .query_filtered::<Entity, With<QuestCard>>()
            .iter(world)
            .collect();
        for card in shown {
            world.entity_mut(card).despawn();
        }
        if let Some((_, moment)) = moment {
            let scene = ui::window(ui::WindowOptions {
                frame: ui::WindowFrame::Anchored {
                    width: Val::Px(WIDTH),
                    height: Val::Auto,
                },
                content: vec![ui::WindowContent {
                    title: moment.title().to_owned(),
                    scene: Box::new(bsn! {
                        QuestCardBody
                        Node { width: Val::Percent(100.0), padding: {UiRect::all(Val::Px(spacing::XL))} }
                    }),
                }],
            });
            if let Some(card) = stage::attach(world, scene) {
                world.entity_mut(card).insert(QuestCard);
            }
        }
    }
    let Some((quest, moment)) = moment else {
        return;
    };
    let Ok(body) = world
        .query_filtered::<Entity, With<QuestCardBody>>()
        .single(world)
    else {
        return;
    };
    let inventory = seen
        .and_then(|seen| world.get::<Inventory>(seen))
        .cloned()
        .unwrap_or_else(Inventory::empty);
    let mut hasher = DefaultHasher::new();
    format!("{:?}", inventory.slots).hash(&mut hasher);
    crate::systems::hud::reconcile_children(world, body, &[hasher.finish()], |world, _| {
        let assets = world.resource::<AssetServer>();
        let parts = match moment {
            Moment::Offer => offer(assets, quest, &inventory),
            Moment::TurnIn => turn_in(assets, quest, &inventory),
            Moment::Decision => decision(quest),
        };
        Box::new(bsn! {
            Node { width: Val::Percent(100.0), flex_direction: FlexDirection::Column, row_gap: Val::Px({spacing::L}) }
            Children [ {parts} ]
        })
    });
}

impl Moment {
    fn title(self) -> &'static str {
        match self {
            Moment::Offer => "Quest offer",
            Moment::TurnIn => "Quest complete",
            Moment::Decision => "A permanent choice",
        }
    }
}

fn moment_of(node: crate::data::dialogue::Id) -> Option<(QuestId, Moment)> {
    <QuestId as strum::VariantArray>::VARIANTS
        .iter()
        .find_map(|&quest| {
            let def = quest.get();
            if node == def.offer {
                Some((quest, Moment::Offer))
            } else if node == def.thanks && def.decision().is_some() {
                Some((quest, Moment::Decision))
            } else if node == def.thanks {
                Some((quest, Moment::TurnIn))
            } else {
                None
            }
        })
}

fn offer(assets: &AssetServer, quest: QuestId, inventory: &Inventory) -> Vec<Box<dyn Scene>> {
    let def = quest.get();
    let ink = ui::theme::theme().surface_floating.on;
    let progress = def.progress(&ActiveQuest::fresh(quest), Some(inventory));
    let mut parts: Vec<Box<dyn Scene>> = vec![
        Box::new(head(assets, def)),
        Box::new(caption(format!("{} · {}", def.kinds(), def.meta()))),
        Box::new(section("Objectives")),
    ];
    parts.extend(def.objectives.iter().zip(&progress).map(
        |(objective, progress)| -> Box<dyn Scene> {
            Box::new(objective_row(assets, objective, progress, ink))
        },
    ));
    parts.push(Box::new(section("Rewards")));
    parts.push(Box::new(reward_chips(assets, def)));
    if !def.pick_one.is_empty() {
        parts.push(Box::new(section("Pick one when you return")));
        parts.push(Box::new(picks(assets, def)));
    }
    parts.push(Box::new(caption("Accept or decline in the conversation")));
    parts
}

fn turn_in(assets: &AssetServer, quest: QuestId, inventory: &Inventory) -> Vec<Box<dyn Scene>> {
    let def = quest.get();
    let mut parts: Vec<Box<dyn Scene>> = vec![Box::new(head(assets, def))];
    parts.extend(def.hand_in.iter().map(|&stack| -> Box<dyn Scene> {
        Box::new(hand_in_row(assets, stack, inventory.count(stack.item)))
    }));
    let pick_note: Vec<Box<dyn Scene>> = (!def.pick_one.is_empty())
        .then(|| -> Box<dyn Scene> { Box::new(caption("+ your pick below")) })
        .into_iter()
        .collect();
    parts.push(Box::new(bsn! {
        Node { column_gap: Val::Px({spacing::L}), align_items: AlignItems::Center, flex_wrap: FlexWrap::Wrap }
        Children [
            {EntityScene(caption("Rewards"))},
            {EntityScene(reward_chips(assets, def))},
            {pick_note},
        ]
    }));
    parts
}

fn decision(quest: QuestId) -> Vec<Box<dyn Scene>> {
    let def = quest.get();
    let ink = ui::theme::theme().surface_floating.on;
    let paths: Vec<Box<dyn Scene>> = def
        .decision()
        .unwrap_or_default()
        .iter()
        .map(|path| -> Box<dyn Scene> {
            let means: Vec<Box<dyn Scene>> = path
                .means
                .iter()
                .map(|mean| -> Box<dyn Scene> {
                    Box::new(ui::styled_text(format!("• {mean}"), ink, typography::BODY))
                })
                .collect();
            Box::new(bsn! {
                Node { flex_direction: FlexDirection::Column, row_gap: Val::Px({spacing::M}), flex_basis: Val::Px(0.0), flex_grow: 1.0 }
                Children [
                    {EntityScene(ui::styled_text(path.choice, ink, typography::LABEL))},
                    {means},
                ]
            })
        })
        .collect();
    vec![
        Box::new(ui::styled_text(def.title, ink, typography::NAME)),
        Box::new(bsn! {
            Node { column_gap: Val::Px({spacing::XL}), width: Val::Percent(100.0) }
            Children [ {paths} ]
        }),
    ]
}

fn head(assets: &AssetServer, def: &QuestDef) -> impl Scene + use<> {
    let ink = ui::theme::theme().surface_floating.on;
    bsn! {
        Node { column_gap: Val::Px({spacing::L}), align_items: AlignItems::Center }
        Children [
            {EntityScene(icon(assets, def.icon(), PICK))},
            {EntityScene(ui::styled_text(def.title, ink, typography::NAME))},
        ]
    }
}

fn hand_in_row(assets: &AssetServer, stack: ItemStack, held: u32) -> impl Scene + use<> {
    let ink = ui::theme::theme().surface_floating.on;
    let held_color = if held >= stack.count {
        ink.with_alpha(0.6)
    } else {
        palette::CRIMSON_80
    };
    bsn! {
        Node { column_gap: Val::Px({spacing::L}), align_items: AlignItems::Center, width: Val::Percent(100.0) }
        Children [
            {EntityScene(caption("Hand in"))},
            {EntityScene(icon(assets, stack.item.get().icon, ICON))},
            ( Node { flex_grow: 1.0 } Children [ {EntityScene(ui::styled_text(counted(stack), ink, typography::BODY))} ] ),
            {EntityScene(ui::styled_text(format!("{held} in your bag"), held_color, typography::CAPTION))},
        ]
    }
}

pub(super) fn objective_row(
    assets: &AssetServer,
    objective: &Objective,
    progress: &Progress,
    color: Color,
) -> impl Scene + use<> {
    let count = if progress.counted {
        format!("{} / {}", progress.have, progress.need)
    } else {
        String::new()
    };
    bsn! {
        Node { column_gap: Val::Px({spacing::L}), align_items: AlignItems::Center, width: Val::Percent(100.0) }
        Children [
            {EntityScene(icon(assets, objective.icon(), ICON))},
            ( Node { flex_grow: 1.0 } Children [ {EntityScene(ui::styled_text(progress.label.clone(), color, typography::BODY))} ] ),
            {EntityScene(ui::styled_text(count, color, typography::LABEL))},
        ]
    }
}

pub(super) fn reward_chips(assets: &AssetServer, def: &QuestDef) -> impl Scene + use<> {
    let ink = ui::theme::theme().surface_floating.on;
    let xp: Vec<Box<dyn Scene>> = (def.xp > 0)
        .then(|| -> Box<dyn Scene> {
            ui::chip(ChipOptions {
                label: format!("{} XP", def.xp),
                icon: None,
                family: Family::outline(ink),
                inspect: None,
            })
        })
        .into_iter()
        .collect();
    let items: Vec<Box<dyn Scene>> = def
        .rewards
        .iter()
        .map(|&stack| -> Box<dyn Scene> {
            ui::chip(ChipOptions {
                label: counted(stack),
                icon: Some(assets.load(stack.item.get().icon.0)),
                family: Family::outline(ink),
                inspect: Some(item::card::inspectable(stack.item)),
            })
        })
        .collect();
    bsn! {
        Node { column_gap: Val::Px({spacing::M}), row_gap: Val::Px({spacing::M}), flex_wrap: FlexWrap::Wrap }
        Children [ {xp}, {items} ]
    }
}

pub(super) fn picks(assets: &AssetServer, def: &QuestDef) -> impl Scene + use<> {
    let slots: Vec<Box<dyn Scene>> = def
        .pick_one
        .iter()
        .map(|stack| -> Box<dyn Scene> {
            let slot = bsn! {
                template_value(crate::systems::hud::slot_node())
                BackgroundColor({crate::systems::hud::SLOT_BG})
                component(BorderColor::all(crate::systems::hud::SLOT_BORDER))
                Children [ {EntityScene(icon(assets, stack.item.get().icon, PICK))} ]
            };
            Box::new(ui::inspectable(item::card::inspectable(stack.item), slot))
        })
        .collect();
    bsn! {
        Node { column_gap: Val::Px({spacing::M}) }
        Children [ {slots} ]
    }
}

pub(super) fn tag(label: &str, color: Color) -> impl Scene + use<> {
    ui::chip(ChipOptions {
        label: label.to_owned(),
        icon: None,
        family: Family::outline(color),
        inspect: None,
    })
}

pub(super) fn section(label: &str) -> impl Scene + use<> {
    let ink = ui::theme::theme().surface_floating.on;
    ui::styled_text(label.to_owned(), ink.with_alpha(0.6), typography::LABEL)
}

pub(super) fn caption(text: impl Into<String>) -> impl Scene {
    let ink = ui::theme::theme().surface_floating.on;
    ui::styled_text(text.into(), ink.with_alpha(0.6), typography::CAPTION)
}

pub(super) fn icon(assets: &AssetServer, icon: AssetRef, size: f32) -> impl Scene + use<> {
    bsn! {
        Node { width: Val::Px({size}), height: Val::Px({size}), flex_shrink: 0.0 }
        component(ImageNode::new(assets.load(icon.0)))
        Pickable::IGNORE
    }
}

pub(super) fn counted(stack: ItemStack) -> String {
    let name = stack.item.get().display_name;
    match stack.count {
        1 => name.to_owned(),
        count => format!("{name} ×{count}"),
    }
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub(super) enum Countdown {
    #[default]
    Left,
    LeftDetail,
    Resets,
    ResetsDetail,
}

impl Countdown {
    fn label(self, log: &QuestLog, quest: QuestId) -> Option<String> {
        match self {
            Countdown::Left => log.active(quest)?.left.map(clock_label),
            Countdown::LeftDetail => log
                .active(quest)?
                .left
                .map(|left| format!("{} left", clock_label(left))),
            Countdown::Resets => log.finished(quest)?.resets_in.map(resets_label),
            Countdown::ResetsDetail => log
                .finished(quest)?
                .resets_in
                .map(|left| format!("Done for today · {}", resets_label(left))),
        }
    }
}

#[derive(Component, Clone, Copy, Default)]
struct QuestClock {
    quest: Option<QuestId>,
    shows: Countdown,
}

#[derive(Component, Clone, Copy, Default)]
struct QuestClockBar {
    quest: Option<QuestId>,
}

pub(super) fn live_text(
    log: &QuestLog,
    quest: QuestId,
    shows: Countdown,
    color: Color,
    typography: ui::tokens::typography::Typography,
) -> impl Scene + use<> {
    let now = shows.label(log, quest).unwrap_or_default();
    bsn! {
        {ui::styled_text(now, color, typography)}
        QuestClock { quest: {Some(quest)}, shows: {shows} }
    }
}

pub(super) fn live_bar(log: &QuestLog, quest: QuestId) -> impl Scene + use<> {
    let (left, limit) = remaining(log, quest);
    bsn! {
        {ui::progress(left, limit)}
        QuestClockBar { quest: {Some(quest)} }
        Children [ {EntityScene(ui::progress_indicator())} ]
    }
}

pub(super) fn steady(log: &QuestLog) -> QuestLog {
    let mut log = log.clone();
    for active in &mut log.active {
        active.left = active.left.map(|_| crate::core::time::Seconds(0.0));
    }
    for finished in &mut log.finished {
        finished.resets_in = finished.resets_in.map(|_| crate::core::time::Seconds(0.0));
    }
    log
}

fn remaining(log: &QuestLog, quest: QuestId) -> (f32, f32) {
    let left = log
        .active(quest)
        .and_then(|active| active.left)
        .map_or(0.0, |left| left.0);
    let limit = quest.get().time_limit.map_or(1.0, |limit| limit.0);
    (left, limit)
}

fn tick_clocks(world: &mut World) {
    let Some(log) = world
        .resource::<Viewpoint>()
        .0
        .and_then(|seen| world.get::<QuestLog>(seen))
        .cloned()
    else {
        return;
    };
    let mut clocks = world.query::<(&QuestClock, &mut Text)>();
    for (clock, mut text) in clocks.iter_mut(world) {
        if let Some(label) = clock.quest.and_then(|quest| clock.shows.label(&log, quest))
            && text.0 != label
        {
            text.0 = label;
        }
    }
    let mut bars = world.query::<(&QuestClockBar, &mut ui::ProgressFraction)>();
    for (bar, mut fraction) in bars.iter_mut(world) {
        if let Some(quest) = bar.quest {
            let (left, limit) = remaining(&log, quest);
            let now = (left / limit).clamp(0.0, 1.0);
            if fraction.0 != now {
                fraction.0 = now;
            }
        }
    }
}
