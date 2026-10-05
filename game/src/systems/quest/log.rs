use std::hash::{DefaultHasher, Hash, Hasher};

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::tokens::{palette, spacing, typography};
use ui::{Check, ConfirmOptions, OnTap};

use super::card::{
    Countdown, caption, icon, live_bar, live_text, objective_row, picks, reward_chips, section,
    steady, tag,
};
use super::{
    ActiveQuest, FinishedQuest, Giver, QUEST_LOG_CAP, QuestId, QuestLog, QuestRequest, QuestResult,
};
use crate::systems::hud::{HudAudience, Window};
use crate::systems::input::map::{ActionInput, InputAction};
use crate::systems::item::widget::slot_note_source;
use crate::systems::item::{Inventory, ItemFlag, ItemStack};
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;
use crate::systems::scene::mode::Mode;

const SIZE: Vec2 = Vec2::new(660.0, 440.0);
const ROW_ICON: f32 = 20.0;

pub struct QuestLogPlugin;

impl Plugin for QuestLogPlugin {
    fn build(&self, app: &mut App) {
        slot_note_source(app, quest_note);
        app.init_resource::<Browse>()
            .add_systems(OnExit(GameScene::Area), forget);
    }
}

pub struct QuestLogWindow;

impl Window for QuestLogWindow {
    fn audience(&self) -> HudAudience {
        HudAudience::Players
    }
    fn title(&self) -> &'static str {
        "Quests"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleQuestLog
    }
    fn icon(&self) -> &'static str {
        "icons/misc/book.png"
    }
    fn order(&self) -> u32 {
        3
    }
    fn size(&self) -> Vec2 {
        SIZE
    }
    fn contents(&self, _: &World) -> Vec<ui::WindowContent> {
        [LogTab::Active, LogTab::Completed]
            .into_iter()
            .map(|tab| ui::WindowContent {
                title: tab.title().into(),
                scene: Box::new(bsn! {
                    QuestLogBody { tab: {tab}, built: None }
                    Node { width: Val::Percent(100.0), height: Val::Percent(100.0) }
                }),
            })
            .collect()
    }
    fn sync(&self, world: &mut World) {
        sync_bodies(world)
    }
}

pub fn select(world: &mut World, tab: LogTab, quest: QuestId) {
    let mut browse = world.resource_mut::<Browse>();
    match tab {
        LogTab::Active => browse.active = Some(quest),
        LogTab::Completed => browse.completed = Some(quest),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LogTab {
    #[default]
    Active,
    Completed,
}

impl LogTab {
    fn title(self) -> &'static str {
        match self {
            LogTab::Active => "Active",
            LogTab::Completed => "Completed",
        }
    }
}

#[derive(Resource, Default)]
struct Browse {
    active: Option<QuestId>,
    completed: Option<QuestId>,
}

impl Browse {
    fn of(&self, tab: LogTab) -> Option<QuestId> {
        match tab {
            LogTab::Active => self.active,
            LogTab::Completed => self.completed,
        }
    }
}

#[derive(Component, Default, Clone)]
struct QuestLogBody {
    tab: LogTab,
    built: Option<u64>,
}

struct Page {
    tab: LogTab,
    log: QuestLog,
    inventory: Inventory,
    selected: Option<QuestId>,
    playing: bool,
}

fn forget(mut browse: ResMut<Browse>) {
    *browse = Browse::default();
}

fn sync_bodies(world: &mut World) {
    let bodies: Vec<(Entity, QuestLogBody)> = world
        .query::<(Entity, &QuestLogBody)>()
        .iter(world)
        .map(|(entity, body)| (entity, body.clone()))
        .collect();
    if bodies.is_empty() {
        return;
    }
    let seen = world.resource::<Viewpoint>().0;
    let log = seen
        .and_then(|seen| world.get::<QuestLog>(seen))
        .cloned()
        .unwrap_or_default();
    let inventory = seen
        .and_then(|seen| world.get::<Inventory>(seen))
        .cloned()
        .unwrap_or_else(Inventory::empty);
    let playing = *world.resource::<Mode>() == Mode::Play;
    for (entity, body) in bodies {
        let page = Page {
            tab: body.tab,
            selected: world.resource::<Browse>().of(body.tab),
            log: log.clone(),
            inventory: inventory.clone(),
            playing,
        };
        let mut hasher = DefaultHasher::new();
        format!(
            "{:?}{:?}{:?}{:?}{playing}",
            page.tab,
            steady(&page.log),
            page.inventory.slots,
            page.selected
        )
        .hash(&mut hasher);
        let key = hasher.finish();
        if body.built == Some(key) {
            continue;
        }
        if let Some(mut body) = world.get_mut::<QuestLogBody>(entity) {
            body.built = Some(key);
        }
        let scene = contents(world, &page);
        world.entity_mut(entity).despawn_related::<Children>();
        if let Ok(mut spawned) = world.spawn_scene(scene) {
            spawned.insert(ChildOf(entity));
        }
    }
}

impl Page {
    fn listed(&self) -> Vec<QuestId> {
        let mut quests: Vec<QuestId> = match self.tab {
            LogTab::Active => self.log.active.iter().map(|active| active.quest).collect(),
            LogTab::Completed => self
                .log
                .finished
                .iter()
                .map(|finished| finished.quest)
                .collect(),
        };
        let first_seen = |category: &str| {
            crate::data::quest::TABLE
                .iter()
                .position(|quest| quest.category == category)
        };
        quests.sort_by_key(|quest| (first_seen(quest.get().category), *quest as usize));
        quests
    }

    fn shown(&self, listed: &[QuestId]) -> Option<QuestId> {
        self.selected
            .filter(|selected| listed.contains(selected))
            .or_else(|| listed.first().copied())
    }

    fn active(&self, quest: QuestId) -> Option<&ActiveQuest> {
        self.log.active(quest)
    }

    fn finished(&self, quest: QuestId) -> Option<&FinishedQuest> {
        self.log.finished(quest)
    }
}

fn contents(world: &World, page: &Page) -> Box<dyn Scene> {
    let assets = world.resource::<AssetServer>();
    let listed = page.listed();
    let shown = page.shown(&listed);
    let (title, aside) = match page.tab {
        LogTab::Active => (
            "Active quests",
            format!("{} / {QUEST_LOG_CAP}", page.log.active.len()),
        ),
        LogTab::Completed => ("Completed", page.log.finished.len().to_string()),
    };
    let mut rows: Vec<Box<dyn Scene>> = vec![Box::new(ui::list_header(title, aside))];
    let mut category = None;
    for &quest in &listed {
        let def = quest.get();
        if category != Some(def.category) {
            category = Some(def.category);
            rows.push(Box::new(group(def.category)));
        }
        rows.push(row(assets, page, quest, shown == Some(quest)));
    }
    if listed.is_empty() {
        rows.push(Box::new(bsn! {
            Node { padding: {UiRect::all(Val::Px(spacing::XL))} }
            Children [ {EntityScene(caption(empty_note(page.tab)))} ]
        }));
    }
    let list: Box<dyn Scene> = Box::new(bsn! {
        Node { flex_direction: FlexDirection::Column, width: Val::Percent(100.0) }
        Children [ {rows} ]
    });
    let detail: Box<dyn Scene> = match shown {
        Some(quest) => Box::new(detail(assets, page, quest)),
        None => Box::new(bsn! { Node }),
    };
    Box::new(ui::split_view(list, Box::new(ui::scrolled(detail))))
}

fn empty_note(tab: LogTab) -> &'static str {
    match tab {
        LogTab::Active => "No quests yet. Look for a ! over someone's head.",
        LogTab::Completed => "Nothing finished yet",
    }
}

fn group(category: &str) -> impl Scene + use<> {
    let ink = ui::theme::theme().surface_floating.on;
    bsn! {
        Node { padding: {UiRect::new(Val::Px(spacing::L), Val::Px(spacing::L), Val::Px(spacing::L), Val::Px(spacing::S))} }
        Children [ {EntityScene(ui::styled_text(category.to_uppercase(), ink.with_alpha(0.5), typography::LABEL))} ]
    }
}

fn row(assets: &AssetServer, page: &Page, quest: QuestId, selected: bool) -> Box<dyn Scene> {
    let ink = ui::theme::theme().surface_floating.on;
    let def = quest.get();
    let status: Vec<Box<dyn Scene>> = vec![status(page, quest)];
    let tracked: Vec<Box<dyn Scene>> = page
        .log
        .tracked
        .contains(&quest)
        .then(|| -> Box<dyn Scene> {
            Box::new(icon(
                assets,
                crate::core::assets::AssetRef("icons/cursors/eye001.png"),
                14.0,
            ))
        })
        .into_iter()
        .collect();
    let background = if selected {
        ui::theme::theme().surface_inset.base
    } else {
        Color::NONE
    };
    let tab = page.tab;
    Box::new(bsn! {
        Node {
            width: Val::Percent(100.0),
            align_items: AlignItems::Center,
            column_gap: Val::Px({spacing::L}),
            padding: {UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::M))},
        }
        BackgroundColor({background})
        Pickable { should_block_lower: true, is_hoverable: true }
        on(move |click: On<Pointer<Click>>, input: ActionInput, mut commands: Commands| {
            if input.clicked(InputAction::Select, &click) {
                commands.queue(move |world: &mut World| select(world, tab, quest));
            }
        })
        Children [
            {EntityScene(icon(assets, def.icon(), ROW_ICON))},
            ( Node { flex_grow: 1.0 } Pickable::IGNORE Children [ {EntityScene(ui::styled_text(def.title, ink, typography::LABEL))} ] ),
            {status},
            {tracked},
        ]
    })
}

fn status(page: &Page, quest: QuestId) -> Box<dyn Scene> {
    let ink = ui::theme::theme().surface_floating.on.with_alpha(0.7);
    let fixed = |text: &str, color: Color| -> Box<dyn Scene> {
        Box::new(ui::styled_text(text.to_owned(), color, typography::CAPTION))
    };
    let live = |shows: Countdown, color: Color| -> Box<dyn Scene> {
        Box::new(live_text(
            &page.log,
            quest,
            shows,
            color,
            typography::CAPTION,
        ))
    };
    let def = quest.get();
    if let Some(active) = page.active(quest) {
        if def.decision().is_some() {
            return fixed("choice", palette::AMBER_80);
        }
        if active.ready {
            return fixed("ready", palette::AMBER_80);
        }
        if active.left.is_some() {
            return live(Countdown::Left, palette::AMBER_80);
        }
        let counted = def
            .progress(active, Some(&page.inventory))
            .into_iter()
            .find(|progress| progress.counted && !progress.done())
            .map(|progress| format!("{}/{}", progress.have, progress.need))
            .unwrap_or_default();
        return fixed(&counted, ink);
    }
    match page.finished(quest) {
        Some(finished) if finished.result == QuestResult::Failed => {
            fixed("failed", palette::CRIMSON_80)
        }
        Some(finished) if finished.resets_in.is_some() => live(Countdown::Resets, ink),
        Some(_) => fixed("done", ink),
        None => fixed("", ink),
    }
}

fn detail(assets: &AssetServer, page: &Page, quest: QuestId) -> impl Scene + use<> {
    let ink = ui::theme::theme().surface_floating.on;
    let def = quest.get();
    let mut parts: Vec<Box<dyn Scene>> = vec![
        Box::new(bsn! {
            Node { column_gap: Val::Px({spacing::L}), align_items: AlignItems::Center, flex_wrap: FlexWrap::Wrap }
            Children [
                {EntityScene(ui::styled_text(def.title, ink, typography::NAME))},
                {EntityScene(tag(&def.kinds(), palette::AMBER_70))},
            ]
        }),
        Box::new(caption(def.meta())),
    ];
    if page
        .active(quest)
        .is_some_and(|active| active.left.is_some())
    {
        parts.push(Box::new(timer(assets, &page.log, quest)));
    }
    parts.push(Box::new(ui::styled_text(
        format!("\u{201c}{}\u{201d}", def.blurb),
        ink.with_alpha(0.85),
        typography::BODY,
    )));
    parts.push(Box::new(section("Objectives")));
    let progress = def.progress(
        page.active(quest).unwrap_or(&ActiveQuest::fresh(quest)),
        Some(&page.inventory),
    );
    let finished = page.finished(quest);
    parts.extend(def.objectives.iter().zip(progress).map(
        |(objective, mut progress)| -> Box<dyn Scene> {
            let color = match finished.map(|finished| finished.result) {
                Some(QuestResult::Completed) => palette::EMERALD_80,
                Some(QuestResult::Failed) => ink.with_alpha(0.6),
                None if progress.done() => palette::EMERALD_80,
                None => ink,
            };
            progress.counted &= finished.is_none();
            Box::new(objective_row(assets, objective, &progress, color))
        },
    ));
    if let Some(paths) = def.decision() {
        parts.push(Box::new(section("Choices")));
        parts.extend(paths.iter().map(|path| -> Box<dyn Scene> {
            Box::new(ui::styled_text(
                format!("{}: {}", path.choice, path.means.join("; ")),
                ink,
                typography::BODY,
            ))
        }));
    }
    parts.push(Box::new(section("Rewards")));
    parts.push(Box::new(reward_chips(assets, def)));
    if !def.pick_one.is_empty() && finished.is_none() {
        parts.push(Box::new(section("Pick one when you return")));
        parts.push(Box::new(picks(assets, def)));
    }
    if def.time_limit.is_some() {
        parts.push(Box::new(section("If time runs out")));
        parts.push(Box::new(ui::styled_text(
            format!("The quest fails. {}.", def.retry_hint()),
            ink,
            typography::BODY,
        )));
    }
    if let Some(finished) = finished {
        parts.push(match finished.result {
            QuestResult::Completed if finished.resets_in.is_some() => Box::new(live_text(
                &page.log,
                quest,
                Countdown::ResetsDetail,
                palette::EMERALD_80,
                typography::LABEL,
            )),
            QuestResult::Completed => Box::new(ui::styled_text(
                "Completed",
                palette::EMERALD_80,
                typography::LABEL,
            )),
            QuestResult::Failed if finished.offered_again => Box::new(ui::styled_text(
                format!("Failed · {}", def.retry_hint()),
                palette::CRIMSON_80,
                typography::LABEL,
            )),
            QuestResult::Failed => Box::new(ui::styled_text(
                "Failed",
                palette::CRIMSON_80,
                typography::LABEL,
            )),
        });
    }
    if page.playing && page.log.trackable(quest) {
        parts.push(Box::new(footer(page, quest)));
    }
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px({spacing::L}),
            padding: {UiRect::all(Val::Px(spacing::XL))},
            width: Val::Percent(100.0),
        }
        Children [ {parts} ]
    }
}

fn timer(assets: &AssetServer, log: &QuestLog, quest: QuestId) -> impl Scene + use<> {
    bsn! {
        Node { column_gap: Val::Px({spacing::L}), align_items: AlignItems::Center, width: Val::Percent(100.0) }
        Children [
            {EntityScene(icon(assets, crate::core::assets::AssetRef("icons/cursors/sandclock001.png"), 16.0))},
            {EntityScene(live_text(log, quest, Countdown::LeftDetail, palette::AMBER_80, typography::LABEL))},
            ( Node { flex_grow: 1.0 } Children [ {EntityScene(live_bar(log, quest))} ] ),
        ]
    }
}

fn footer(page: &Page, quest: QuestId) -> impl Scene + use<> {
    let ink = ui::theme::theme().surface_floating.on;
    let tracked = page.log.tracked.contains(&quest);
    let check = if tracked { Check::On } else { Check::Off };
    let abandon: Vec<Box<dyn Scene>> = page
        .active(quest)
        .map(|_| -> Box<dyn Scene> {
            Box::new(bsn! {
                {ui::button_styled(ui::button::intent::DANGER, ui::ButtonSize::Sm, "Abandon")}
                on(move |_: On<ui::Activate>, mut commands: Commands| {
                    commands.queue(move |world: &mut World| confirm_abandon(world, quest));
                })
            })
        })
        .into_iter()
        .collect();
    bsn! {
        Node {
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            width: Val::Percent(100.0),
            padding: {UiRect::top(Val::Px(spacing::L))},
        }
        Children [
            (
                Node { column_gap: Val::Px({spacing::L}), align_items: AlignItems::Center }
                Children [
                    (
                        {ui::checkbox(check)}
                        on(move |change: On<ui::ValueChange<bool>>, mut commands: Commands| {
                            let tracked = change.value;
                            commands.queue(move |world: &mut World| {
                                world.write_message(QuestRequest::Track { quest, tracked });
                            });
                        })
                        Children [ {EntityScene(ui::checkbox_indicator())} ]
                    ),
                    (
                        {ui::styled_text("Track on HUD", ink, typography::BODY)}
                        Pickable { should_block_lower: true, is_hoverable: true }
                        on(move |click: On<Pointer<Click>>, input: ActionInput, mut commands: Commands| {
                            if input.clicked(InputAction::Select, &click) {
                                commands.queue(move |world: &mut World| {
                                    world.write_message(QuestRequest::Track { quest, tracked: !tracked });
                                });
                            }
                        })
                    ),
                ]
            ),
            {abandon},
        ]
    }
}

fn confirm_abandon(world: &mut World, quest: QuestId) {
    let def = quest.get();
    let taken = def
        .grants
        .iter()
        .map(|stack| format!("{} is taken from your bag.", stack.item.get().display_name));
    let again = match def.giver {
        Giver::Item(item) => format!("Use the {} to start it again.", item.get().display_name),
        giver => format!("{} will offer the quest again.", giver.name()),
    };
    let body = taken
        .chain(["Your progress is lost.".to_owned(), again])
        .collect();
    let dialog = ui::confirm_dialog(ConfirmOptions {
        title: format!("Abandon {}?", def.title),
        body,
        confirm: "Abandon quest".to_owned(),
        cancel: "Keep quest".to_owned(),
        on_confirm: OnTap::new(move |world| {
            world.write_message(QuestRequest::Abandon { quest });
        }),
        on_cancel: OnTap::new(|_| {}),
        keep: InputAction::TakeDefault.into(),
    });
    world.spawn_scene(dialog).ok();
}

fn quest_note(world: &World, stack: ItemStack) -> Option<String> {
    let quests = <QuestId as strum::VariantArray>::VARIANTS;
    if quests
        .iter()
        .any(|quest| quest.get().giver == Giver::Item(stack.item))
    {
        return Some("begins a quest · use to read it".to_owned());
    }
    if !stack.item.get().has(ItemFlag::Quest) {
        return None;
    }
    let granted_by = world
        .resource::<Viewpoint>()
        .0
        .and_then(|seen| world.get::<QuestLog>(seen))
        .and_then(|log| {
            log.active.iter().find(|active| {
                active
                    .quest
                    .get()
                    .grants
                    .iter()
                    .any(|grant| grant.item == stack.item)
            })
        })
        .map(|active| active.quest.get().title);
    Some(match granted_by {
        Some(title) => {
            format!("quest item · can't be sold or dropped · taken back if you abandon {title}")
        }
        None => "quest item · can't be sold or dropped".to_owned(),
    })
}
