use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::card::{self, Card, CardAction, CardResponse, CardTilt, Plate};
use ui::controls::{self, Pill};
use ui::egui::{Align, Label, Layout, Rect, Response, Sense, Ui};
use ui::face::{self, TextRole};
use ui::glyph::{self, CardKind, Mark};
use ui::ink;
use ui::motion;

use super::{
    ActiveQuest, Objective, ObjectiveProgress, Parties, QuestDef, QuestId, QuestLog, QuestResult,
    clock_label, resets_label,
};
use crate::core::card::CardStyleDef;
use crate::core::content::Content;
use crate::core::gui::{Band, ImageSource};
use crate::core::stage::card::{self as card_stage, HungCardLamp, StagedCard};
use crate::core::time::UnixMillis;
use crate::systems::chat::ChatEmbed;
use crate::systems::dialogue::stage::{self, DockedPanel, PanelChrome, PanelHead};
use crate::systems::dialogue::{ChoiceReveal, Conversation, ConversationPanel};
use crate::systems::hud;
use crate::systems::item::card::CardLook;
use crate::systems::item::widget::{SlotLook, slot};
use crate::systems::item::{self, Inventory, ItemId, ItemStack};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::settings::UserSettings;
use crate::systems::terminal::chat;

const ICON: f32 = 20.0;
const MARK: f32 = 13.0;
const CHIP_ICON: f32 = 16.0;

pub(crate) const WINDOW_KEY: &str = "Quest";

pub struct QuestCardPlugin;

impl Plugin for QuestCardPlugin {
    fn build(&self, app: &mut App) {
        stage::dock(
            app,
            DockedPanel {
                head: card_head,
                show: show_docked,
            },
        );
        app.init_resource::<hud::Pinned<QuestId>>()
            .add_systems(
                Update,
                hud::raise_pinned::<QuestId>(WINDOW_KEY).run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                draw_card
                    .in_set(Band::Hud)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), hud::forget_pinned::<QuestId>);
    }
}

/// Pins the card of `quest`, dealt from `from`.
pub fn open(world: &mut World, quest: QuestId, from: Option<Rect>) {
    hud::pin(world, quest, from);
}

pub fn close(world: &mut World) -> bool {
    hud::unpin::<QuestId>(world)
}

/// How far the pinned card is turned toward the pointer.
pub fn tilt(world: &World) -> CardTilt {
    hud::pinned_tilt(world, WINDOW_KEY)
}

/// The lamp the pinned card is lit by, while it is on the stage.
pub fn lamp(world: &World) -> Option<HungCardLamp> {
    hud::pinned_lamp(world, WINDOW_KEY)
}

/// The quest whose card is pinned, and where the card is drawn.
pub fn shown(world: &World) -> Option<(QuestId, Rect)> {
    Some((
        hud::pinned::<QuestId>(world)?,
        hud::pinned_rect::<QuestId>(world)?,
    ))
}

/// How a quest's card is shown: at which `moment` of a conversation, when at one, what `action`
/// its band offers, and how it stands among the windows.
pub struct QuestCardLook<'a> {
    pub moment: Option<QuestMoment>,
    pub action: Option<CardAction<'a>>,
    pub look: CardLook,
}

/// A quest's card: its face, and whatever `plate` writes under the overview.
pub fn show<I: ImageSource + ?Sized>(
    ui: &mut Ui,
    images: &mut I,
    content: &Content,
    quest: QuestId,
    shown_as: QuestCardLook<'_>,
    plate: impl FnOnce(&mut Ui, &mut I, Plate),
) -> CardResponse {
    let QuestCardLook {
        moment,
        action,
        look,
    } = shown_as;
    let def = quest.get(content);
    let band = match moment {
        Some(moment) => format!("{} · {}", moment.band(), def.category.to_lowercase()),
        None => def.category.to_lowercase(),
    };
    let icon = images.image(def.icon(content)).map(|image| image.texture);
    let mut shown = Card::new(CardKind::Quest, &def.title);
    shown.dress = CardStyleDef::of(content, def.card_style).dress(images, None);
    shown.band = &band;
    shown.picture = icon;
    shown.action = action;
    shown.closable = look.closable;
    shown.depth = look.depth;
    shown.show(ui, |ui, written| {
        if written.name_cut {
            card::full_name(ui, &def.title);
        }
        card::overview(ui, &overview(def));
        plate(ui, images, written);
    })
}

/// The body of a quest's offer: what it asks, and what it gives when the quest lets on. Answers
/// the rewards shown, for a reader who may inspect them.
pub fn offer<I: ImageSource + ?Sized>(
    ui: &mut Ui,
    images: &mut I,
    content: &Content,
    parties: &Parties,
    quest: QuestId,
    inventory: Option<&Inventory>,
    plate: Plate,
) -> Vec<(Response, ItemId)> {
    let reader = QuestReader {
        log: &QuestLog::default(),
        inventory,
    };
    let shown = terms(ui, images, content, parties, quest, reader, plate);
    card::foot(ui, "Accept or decline in the conversation");
    shown
}

/// Who reads a quest's card: the quests they hold and have finished, and what they carry.
#[derive(Clone, Copy)]
struct QuestReader<'a> {
    log: &'a QuestLog,
    inventory: Option<&'a Inventory>,
}

/// What a quest asks, as far as its reader is with it, and what it gives when the quest lets
/// on. Answers the rewards shown, for a reader who may inspect them.
fn terms<I: ImageSource + ?Sized>(
    ui: &mut Ui,
    images: &mut I,
    content: &Content,
    parties: &Parties,
    quest: QuestId,
    reader: QuestReader<'_>,
    plate: Plate,
) -> Vec<(Response, ItemId)> {
    let def = quest.get(content);
    let fresh = ActiveQuest::fresh(content, quest);
    let progress = def.progress(
        content,
        parties,
        reader.log.active(quest).unwrap_or(&fresh),
        reader.inventory,
    );
    let finished = reader.log.finished(quest).map(|finished| finished.result);
    let standing = match finished {
        Some(QuestResult::Completed) => Standing::Done,
        Some(QuestResult::Failed) => Standing::Failed,
        None => Standing::Open,
    };
    card::overview(ui, &def.meta(parties, quest));
    card::heading(ui, plate, "objectives");
    for (index, (objective, mut progress)) in def.objectives().zip(progress).enumerate() {
        progress.counted &= finished.is_none();
        ui.push_id(("objective", index), |ui| {
            objective_row(ui, images, content, objective, &progress, standing);
        });
    }
    let mut shown = Vec::new();
    if def.reveals(ChoiceReveal::Gains) {
        card::heading(ui, plate, "rewards");
        shown.extend(reward_chips(ui, images, content, def));
        if !def.pick_one().is_empty() && finished.is_none() {
            card::heading(ui, plate, "pick one when you return");
            shown.extend(picks(ui, images, content, def));
        }
    }
    shown
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestMoment {
    Offer,
    TurnIn,
    Decision,
}

impl QuestMoment {
    pub fn band(self) -> &'static str {
        match self {
            QuestMoment::Offer => "quest offer",
            QuestMoment::TurnIn => "quest complete",
            QuestMoment::Decision => "a permanent choice",
        }
    }
}

fn moment(world: &World) -> Option<(QuestId, QuestMoment)> {
    let seen = session::me(world)?;
    let panel = world.get::<Conversation>(seen)?.panel.as_ref()?;
    Some(match *panel {
        ConversationPanel::QuestOffer(quest) => (quest, QuestMoment::Offer),
        ConversationPanel::QuestTurnIn(quest) => (quest, QuestMoment::TurnIn),
        ConversationPanel::QuestDecision(quest) => (quest, QuestMoment::Decision),
    })
}

fn card_head(world: &World) -> Option<PanelHead> {
    moment(world)?;
    Some(PanelHead {
        width: card::WIDTH,
        chrome: PanelChrome::Card,
    })
}

fn show_docked(ui: &mut Ui, world: &mut World, _: f32, _: f32) {
    let Some((quest, moment)) = moment(world) else {
        return;
    };
    let inventory = session::me(world)
        .and_then(|seen| world.get::<Inventory>(seen))
        .cloned()
        .unwrap_or_else(Inventory::empty);
    let content = world.resource::<Content>().clone();
    let parties = world.resource::<Parties>().clone();
    let painted = ui.layer_id();
    let since = motion::mark(ui.ctx(), painted);
    let shown_as = QuestCardLook {
        moment: Some(moment),
        action: Some(chat::card_action()),
        look: CardLook::DOCKED,
    };
    let shown = show(ui, world, &content, quest, shown_as, |ui, world, plate| {
        let shown = match moment {
            QuestMoment::Offer => offer(
                ui,
                world,
                &content,
                &parties,
                quest,
                Some(&inventory),
                plate,
            ),
            QuestMoment::TurnIn => turn_in(ui, world, &content, quest, &inventory, plate),
            QuestMoment::Decision => {
                decision(ui, &content, quest);
                Vec::new()
            }
        };
        inspectable(world, shown);
    });
    chat::offered(world, &shown, ChatEmbed::Quest(quest));
    let staged = StagedCard {
        key: ui.id().with("docked quest card"),
        runs: motion::lifted(ui.ctx(), painted, since),
        flat: shown.rect,
        pose: None,
        turns: world.resource::<UserSettings>().cards_tilt(),
        material: CardStyleDef::of(&content, quest.get(&content).card_style)
            .material
            .clone(),
    };
    card_stage::stage(world, ui.ctx(), staged, painted);
}

fn draw_card(world: &mut World) {
    let material = |world: &World, quest: &QuestId| {
        let content = world.resource::<Content>();
        CardStyleDef::of(content, quest.get(content).card_style)
            .material
            .clone()
    };
    hud::draw_pinned::<QuestId>(world, WINDOW_KEY, material, |ui, world, quest, depth| {
        let quest = *quest;
        let content = world.resource::<Content>().clone();
        let parties = world.resource::<Parties>().clone();
        let me = session::me(world);
        let log = me
            .and_then(|me| world.get::<QuestLog>(me))
            .cloned()
            .unwrap_or_default();
        let inventory = me.and_then(|me| world.get::<Inventory>(me)).cloned();
        let shown_as = QuestCardLook {
            moment: None,
            action: Some(chat::card_action()),
            look: CardLook {
                depth,
                ..CardLook::PINNED
            },
        };
        let shown = show(ui, world, &content, quest, shown_as, |ui, world, plate| {
            let reader = QuestReader {
                log: &log,
                inventory: inventory.as_ref(),
            };
            let shown = terms(ui, world, &content, &parties, quest, reader, plate);
            inspectable(world, shown);
        });
        chat::offered(world, &shown, ChatEmbed::Quest(quest));
        shown
    });
}

/// Lets the reader inspect each item a card shows.
pub(super) fn inspectable(world: &mut World, shown: Vec<(Response, ItemId)>) {
    for (response, item) in shown {
        item::card::inspect(world, response, item);
    }
}

fn turn_in(
    ui: &mut Ui,
    images: &mut World,
    content: &Content,
    quest: QuestId,
    inventory: &Inventory,
    plate: Plate,
) -> Vec<(Response, ItemId)> {
    let def = quest.get(content);
    if def.reveals(ChoiceReveal::Costs) {
        card::heading(ui, plate, "hand in");
        for (index, &stack) in def.hand_in().iter().enumerate() {
            ui.push_id(("hand in", index), |ui| {
                hand_in_row(ui, images, content, stack, inventory.count(stack.item));
            });
        }
    }
    if !def.reveals(ChoiceReveal::Gains) {
        return Vec::new();
    }
    card::heading(ui, plate, "rewards");
    let shown = reward_chips(ui, images, content, def);
    if !def.pick_one().is_empty() {
        card::foot(ui, "Your pick is offered in the conversation");
    }
    shown
}

fn decision(ui: &mut Ui, content: &Content, quest: QuestId) {
    let def = quest.get(content);
    let paths = def.decision().unwrap_or_default();
    ui.columns(paths.len().max(1), |columns| {
        for (column, path) in columns.iter_mut().zip(paths) {
            column.label(face::bold(&path.choice).size(TextRole::CardBody.size()));
            for mean in &path.means {
                column.horizontal_wrapped(|ui| {
                    glyph::show(ui, Mark::Dot, 8.0, ink::INK2);
                    ui.add(Label::new(TextRole::CardBody.rich(mean.as_str())).wrap());
                });
            }
        }
    });
}

fn hand_in_row(ui: &mut Ui, images: &mut World, content: &Content, stack: ItemStack, held: u32) {
    let short = held < stack.count;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let color = if short { ink::RUBRIC } else { ink::INK3 };
            ui.label(
                TextRole::Small
                    .rich(format!("{held} in your inventory"))
                    .color(color),
            );
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                hud::icon(ui, images, &stack.item.get(content).icon, ICON);
                ui.add(Label::new(TextRole::CardBody.rich(counted(content, stack))).wrap());
            });
        });
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Standing {
    Open,
    Done,
    Failed,
}

pub(super) fn objective_row(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    objective: &Objective,
    progress: &ObjectiveProgress,
    standing: Standing,
) {
    let standing = match standing {
        Standing::Open if progress.done() => Standing::Done,
        other => other,
    };
    let (mark, color) = match standing {
        Standing::Open => (Mark::Flag, ink::INK),
        Standing::Done => (Mark::Check, ink::INK),
        Standing::Failed => (Mark::Cross, ink::INK3),
    };
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if progress.counted {
                ui.label(
                    TextRole::CardBody
                        .rich(format!("{} / {}", progress.have, progress.need))
                        .color(color),
                );
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                glyph::show(ui, mark, MARK, color);
                if let Some(icon) = objective.icon(content) {
                    hud::icon(ui, images, icon, ICON);
                }
                ui.add(
                    Label::new(
                        TextRole::CardBody
                            .rich(progress.label.plain_words())
                            .color(color),
                    )
                    .wrap(),
                );
            });
        });
    });
}

pub(super) fn reward_chips(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    def: &QuestDef,
) -> Vec<(Response, ItemId)> {
    ui.horizontal_wrapped(|ui| {
        if def.xp > 0 {
            controls::pill(ui, &format!("{} XP", def.xp), Pill::Gold);
        }
        def.rewards()
            .iter()
            .map(|&stack| (item_chip(ui, images, content, stack), stack.item))
            .collect()
    })
    .inner
}

pub(super) fn picks(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    def: &QuestDef,
) -> Vec<(Response, ItemId)> {
    ui.horizontal(|ui| {
        def.pick_one()
            .iter()
            .enumerate()
            .map(|(index, stack)| {
                let look = SlotLook {
                    count: stack.count,
                    ..SlotLook::of(content, stack.item)
                };
                let response = ui
                    .push_id(("pick", index), |ui| slot(ui, images, look, Sense::click()))
                    .inner;
                (response, stack.item)
            })
            .collect()
    })
    .inner
}

pub(super) fn caption(ui: &mut Ui, text: impl Into<String>) {
    ui.add(Label::new(face::small_faint(text.into())).wrap());
}

pub(super) fn overview(def: &QuestDef) -> String {
    let kinds = def.kinds();
    let mut letters = kinds.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

fn counted(content: &Content, stack: ItemStack) -> String {
    let name = &stack.item.get(content).name;
    match stack.count {
        1 => name.to_owned(),
        count => format!("{name} \u{d7}{count}"),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Countdown {
    #[default]
    Left,
    LeftDetail,
    Resets,
    ResetsDetail,
}

impl Countdown {
    pub(super) fn label(self, log: &QuestLog, quest: QuestId, now: UnixMillis) -> Option<String> {
        match self {
            Countdown::Left => log.active(quest)?.left(now).map(clock_label),
            Countdown::LeftDetail => log
                .active(quest)?
                .left(now)
                .map(|left| format!("{} left", clock_label(left))),
            Countdown::Resets => log.finished(quest)?.resets_in(now).map(resets_label),
            Countdown::ResetsDetail => log
                .finished(quest)?
                .resets_in(now)
                .map(|left| format!("Done for today · {}", resets_label(left))),
        }
    }
}

pub(super) fn clock_now() -> UnixMillis {
    crate::core::time::LocalClock::of_this_machine().now
}

pub(super) fn time_left(content: &Content, log: &QuestLog, quest: QuestId, now: UnixMillis) -> f32 {
    let left = log
        .active(quest)
        .and_then(|active| active.left(now))
        .map_or(0.0, |left| left.0);
    let limit = quest.get(content).time_limit().map_or(1.0, |limit| limit.0);
    (left / limit).clamp(0.0, 1.0)
}

fn item_chip(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    stack: ItemStack,
) -> Response {
    controls::pill_with(ui, Pill::Gold, |ui| {
        hud::icon(ui, images, &stack.item.get(content).icon, CHIP_ICON);
        ui.label(counted(content, stack));
    })
    .response
    .interact(Sense::click())
}
