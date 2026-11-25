mod card;
pub mod day_clock;
pub mod menu;
mod panel;

pub(crate) use card::{
    Pinned, draw_pinned, forget_pinned, pin, pinned, pinned_lamp, pinned_rect, pinned_tilt,
    raise_pinned, unpin,
};
pub(crate) use panel::{
    Centred, Seat, SizedPiece, Sizing, WindowKey, centred, drawn_rect, grab_sized, grab_spot,
    mark_drawn, panel_id, placed_spot, sheet_pose, sized_rect, tabs, viewed, widget,
};

use std::collections::HashSet;
use std::ops::RangeInclusive;
use ui::dialog;
use ui::drag;
use ui::egui;
use ui::float;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::egui::{Align2, Color32, Id, LayerId, Pos2, Rect, Sense, Ui, Vec2};
use ui::face::{self, TextRole};
use ui::{controls, ink, paint, sheet};

use crate::core::content::Content;
use crate::core::content::ImageRef;
use crate::core::gui::{self, Band, ImageSource, WidgetRects};
use crate::core::text::LineText;
use crate::core::text::{self, StyledLine, TextLook};
use crate::systems::area_map::minimap;
use crate::systems::input::map::{InputAction, InputMap};
use crate::systems::item::inline;
use crate::systems::item::transfer::TransferId;
use crate::systems::item::widget::SlotRarity;
use crate::systems::item::{ItemId, ItemStack};
use crate::systems::messages::{self, Messages, messages};
use crate::systems::player::{self, session};
use crate::systems::screen::ClientScreen;
use crate::systems::skill::{HotbarEntry, HotbarSlot, SkillId};
use crate::systems::{
    area_instance, equipment, item, market, mount, party, quest, social_request, trade,
};

pub(crate) const TOOLTIP_WIDTH: f32 = 260.0;
pub(crate) const MARGIN: f32 = 8.0;
const CARRIED: f32 = 40.0;
const NOTE: Vec2 = Vec2::new(400.0, 170.0);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        menu::plugin(app);
        app.init_resource::<panel::Arranging>()
            .init_resource::<panel::SheetClocks>()
            .init_resource::<panel::Afterimages>()
            .init_resource::<WindowsDrawn>()
            .init_resource::<Confirming>()
            .init_resource::<Lifting>()
            .init_resource::<DropHint>()
            .add_systems(
                EguiPrimaryContextPass,
                (
                    (forget_drawn, panel::arrange).before(Band::Stage),
                    draw_hud
                        .in_set(Band::Hud)
                        .run_if(in_state(ClientScreen::Area)),
                    draw_confirm
                        .in_set(Band::Modal)
                        .run_if(resource_exists::<Content>),
                    draw_carried
                        .in_set(Band::Pointer)
                        .run_if(in_state(ClientScreen::Area)),
                ),
            );
    }
}

pub trait HudWidget: Send + Sync {
    fn fallback(&self, world: &World, screen: Rect) -> Pos2;
    /// The corner of the widget that rests at its fallback, and so stays put as it grows.
    fn rests_by(&self) -> Align2 {
        Align2::LEFT_TOP
    }
    fn hidden(&self, _world: &World) -> bool {
        false
    }
    // Returns the room of what it drew outside its own, so that whatever keeps clear of the
    // interface keeps clear of that too.
    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<Rect>;
}

/// A question put to the player, answered with `true` for the confirming button and `false` for the
/// cancelling one or a dismissal.
pub(crate) struct Confirm {
    pub title: LineText,
    pub body: Vec<LineText>,
    pub confirm: String,
    pub cancel: String,
    pub answer: Answer,
}

pub(crate) type Answer = Box<dyn FnOnce(&mut World, bool) + Send + Sync>;

/// Something the player is told, which they dismiss with `dismiss`.
pub(crate) struct Tell {
    pub title: LineText,
    pub body: Vec<LineText>,
    pub dismiss: String,
}

/// A question of how many, put to the player with a slider over `amounts`: answered with the
/// amount chosen for the confirming button, and with none for the cancelling one or a dismissal.
pub(crate) struct AmountQuestion {
    pub title: LineText,
    pub amounts: RangeInclusive<u32>,
    /// The amount the slider starts on.
    pub suggested: u32,
    /// What choosing an amount comes to, said beside the slider as it moves.
    pub says: AmountWording,
    pub answer: AmountAnswer,
}

pub(crate) type AmountWording = Box<dyn Fn(u32) -> String + Send + Sync>;
pub(crate) type AmountAnswer = Box<dyn FnOnce(&mut World, Option<u32>) + Send + Sync>;

/// What the player drags across the interface, drawn under the pointer until it is dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Carried {
    Skill(SkillId),
    BagSlot(u32),
    Worn(equipment::EquipmentPlace),
    BarEntry {
        from: HotbarSlot,
        entry: HotbarEntry,
    },
    /// The stack at `index` of the slots of the window `from` names, one items move between.
    Moved {
        from: TransferId,
        index: usize,
        stack: ItemStack,
    },
}

pub fn icon(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    asset: &ImageRef,
    size: f32,
) -> egui::Response {
    sized(ui, images.image(asset), size)
}

pub(crate) fn confirm(world: &mut World, confirm: Confirm) {
    world.resource_mut::<Confirming>().0 = Some(Pending {
        asked: Asked::Confirm(confirm),
        focused: false,
    });
}

pub(crate) fn tell(world: &mut World, tell: Tell) {
    world.resource_mut::<Confirming>().0 = Some(Pending {
        asked: Asked::Tell(tell),
        focused: false,
    });
}

pub(crate) fn ask_amount(world: &mut World, question: AmountQuestion) {
    let chosen = question
        .suggested
        .clamp(*question.amounts.start(), *question.amounts.end());
    world.resource_mut::<Confirming>().0 = Some(Pending {
        asked: Asked::Amount { question, chosen },
        focused: false,
    });
}

pub(crate) fn confirming(world: &World) -> bool {
    world.resource::<Confirming>().0.is_some()
}

pub(crate) fn dismiss_confirm(world: &mut World) -> bool {
    match world.resource_mut::<Confirming>().0.take() {
        Some(pending) => {
            pending.asked.answer(world, false);
            true
        }
        None => false,
    }
}

pub(crate) fn is_piece(name: &str) -> bool {
    WIDGETS.iter().any(|&(piece, _)| piece == name)
}

pub(crate) fn keeps_place(name: &str) -> bool {
    let cards = [
        WindowKey::ItemCard,
        WindowKey::SkillCard,
        WindowKey::QuestCard,
        WindowKey::QuestOffer,
    ];
    is_piece(name) || cards.iter().any(|card| card.name() == name)
}

pub(crate) fn window_layer(key: WindowKey) -> LayerId {
    panel::layer(key)
}

pub(crate) fn confirm_layer() -> LayerId {
    LayerId::new(Band::Modal.order(), Id::new(WindowKey::Confirm.name()))
}

/// Where the widget of the interface named `name` stands, while it shows.
pub(crate) fn shown_widget_rect(world: &World, name: &str) -> Option<Rect> {
    let (_, widget) = WIDGETS.iter().find(|(key, _)| *key == name)?;
    if widget.hidden(world) {
        return None;
    }
    world.resource::<WidgetRects>().0.get(name).copied()
}

/// Where every widget and window of the interface stood when last drawn.
pub(crate) fn drawn_rects(world: &World) -> Vec<Rect> {
    world
        .resource::<WidgetRects>()
        .0
        .values()
        .copied()
        .collect()
}

pub(crate) fn tooltip(
    response: &egui::Response,
    world: &World,
    title: &str,
    lines: &[LineText],
    hint: Option<&StyledLine>,
) {
    tooltip_marked(response, world, Some(title), None, lines, hint);
}

/// A tooltip whose first line is led by `mark`, a small picture of what the line says. One with
/// no `title` leaves naming what it is about to where it is shown.
pub(crate) fn tooltip_marked(
    response: &egui::Response,
    world: &World,
    title: Option<&str>,
    mark: Option<&gui::GuiImage>,
    lines: &[LineText],
    hint: Option<&StyledLine>,
) {
    float::tooltip(response, |ui| {
        ui.set_max_width(TOOLTIP_WIDTH);
        ui.spacing_mut().item_spacing.y = 2.0;
        if let Some(title) = title {
            ui.label(face::bold(title).size(TextRole::Tooltip.size()));
        }
        for (index, line) in lines.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if index == 0
                    && let Some(mark) = mark
                {
                    ui.add(mark.widget(Vec2::splat(TextRole::Tooltip.size())));
                }
                let look = TextLook::new(TextRole::Tooltip.size(), ink::INK2);
                text::show(
                    ui,
                    world,
                    &line.styled(world.resource::<Content>()),
                    look,
                    None,
                );
            });
        }
        if let Some(hint) = hint {
            let look = TextLook::new(TextRole::Small.size(), ink::INK3);
            text::show(ui, world, hint, look, None);
        }
    });
}

static WIDGETS: &[(&str, &dyn HudWidget)] = &[
    (minimap::MINIMAP, &minimap::MinimapWidget),
    (
        quest::tracker::QUEST_TRACKER,
        &quest::tracker::QuestTrackerWidget,
    ),
    (player::bar::PLAYER_BAR, &player::bar::PlayerBarWidget),
    (
        social_request::card::REQUEST_CARDS,
        &social_request::card::RequestCardsWidget,
    ),
    (
        party::frames::PARTY_FRAMES,
        &party::frames::PartyFramesWidget,
    ),
    (mount::plate::MOUNT_PLATE, &mount::plate::MountPlateWidget),
    (
        area_instance::gather::INSTANCE_CALL,
        &area_instance::gather::CallCardWidget,
    ),
];

#[derive(Resource, Default)]
struct WindowsDrawn {
    last: HashSet<WindowKey>,
    now: HashSet<WindowKey>,
}

#[derive(Resource, Default)]
struct Confirming(Option<Pending>);

/// What the confirm note says, kept while the note lifts off after it is answered.
#[derive(Resource, Default)]
struct Lifting(Option<NoteText>);

#[derive(Clone)]
struct NoteText {
    title: LineText,
    body: Vec<LineText>,
    /// The confirming button, where the note asks something rather than tells it.
    confirm: Option<String>,
    cancel: String,
    amount: Option<NoteAmount>,
}

#[derive(Clone)]
struct NoteAmount {
    chosen: u32,
    amounts: RangeInclusive<u32>,
}

impl NoteText {
    fn of(asked: &Asked, messages: &Messages) -> NoteText {
        match asked {
            Asked::Confirm(confirm) => NoteText {
                title: confirm.title.clone(),
                body: confirm.body.clone(),
                confirm: Some(confirm.confirm.clone()),
                cancel: confirm.cancel.clone(),
                amount: None,
            },
            Asked::Amount { question, chosen } => NoteText {
                title: question.title.clone(),
                body: vec![(question.says)(*chosen).into()],
                confirm: Some(messages.hud.confirm().to_owned()),
                cancel: messages.interface.cancel().to_owned(),
                amount: Some(NoteAmount {
                    chosen: *chosen,
                    amounts: question.amounts.clone(),
                }),
            },
            Asked::Tell(tell) => NoteText {
                title: tell.title.clone(),
                body: tell.body.clone(),
                confirm: None,
                cancel: tell.dismiss.clone(),
                amount: None,
            },
        }
    }
}

struct Pending {
    asked: Asked,
    focused: bool,
}

enum Asked {
    Confirm(Confirm),
    Amount {
        question: AmountQuestion,
        chosen: u32,
    },
    Tell(Tell),
}

impl Asked {
    fn answer(self, world: &mut World, confirmed: bool) {
        match self {
            Asked::Confirm(confirm) => (confirm.answer)(world, confirmed),
            Asked::Amount { question, chosen } => {
                (question.answer)(world, confirmed.then_some(chosen));
            }
            Asked::Tell(_) => {}
        }
    }
}

/// The menu and the cards that are up, as they are stacked, the one in front first.
pub(crate) fn sheets_front_first(world: &World) -> Vec<WindowKey> {
    gui::context(world).map_or_else(Vec::new, |gui| panel::sheets_front_first(world, &gui))
}

fn forget_drawn(mut drawn: ResMut<WindowsDrawn>) {
    let drawn = &mut *drawn;
    drawn.last = std::mem::take(&mut drawn.now);
}

fn draw_hud(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let screen = gui.content_rect();
    for &(name, shown) in WIDGETS {
        if shown.hidden(world) {
            continue;
        }
        let rest = (shown.rests_by(), shown.fallback(world, screen));
        let past = widget(world, &gui, name, rest, |ui, world| shown.show(ui, world));
        if let Some(past) = past
            && let Some(drawn) = world.resource_mut::<WidgetRects>().0.get_mut(name)
        {
            *drawn = drawn.union(past);
        }
    }
    area_instance::gather::draw(world, &gui);
    trade::window::draw(world, &gui);
    market::window::draw(world, &gui);
    menu::draw(world, &gui);
}

fn draw_confirm(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let pending = world.resource_mut::<Confirming>().0.take();
    let content = world.resource::<Content>().clone();
    let messages = messages::of(&content);
    let shown_text = pending
        .as_ref()
        .map(|pending| NoteText::of(&pending.asked, messages));
    if let Some(text) = &shown_text {
        world.resource_mut::<Lifting>().0 = Some(text.clone());
    }
    let Some(pose) = panel::sheet_pose(world, &gui, WindowKey::Confirm, pending.is_some()) else {
        world.resource_mut::<Lifting>().0 = None;
        return;
    };
    let Some(text) = shown_text.or_else(|| world.resource::<Lifting>().0.clone()) else {
        return;
    };
    let default_key = world
        .resource::<InputMap>()
        .shortcut(InputAction::TakeDefault);
    let mut pending = pending;
    let mut decided = None;
    let screen = gui.content_rect();
    let rest = Rect::from_center_size(screen.center(), NOTE);
    gui.set_transform_layer(confirm_layer(), pose.transform(&gui, rest, rest.min));
    let modal = egui::Modal::new(Id::new(WindowKey::Confirm.name()))
        .backdrop_color(ink::ink_wash(0.3 * pose.opacity))
        .frame(egui::Frame::NONE)
        .show(&gui, |ui| {
            ui.multiply_opacity(pose.opacity);
            if pending.is_none() {
                ui.disable();
            }
            let padding = egui::Margin::same(dialog::PADDING);
            sheet::note_shadowed(ui, padding, pose.shadow, |ui| {
                let padding = 2.0 * f32::from(dialog::PADDING);
                ui.set_width(NOTE.x - padding);
                let ink = ui.visuals().text_color();
                let title = TextLook::role(TextRole::NoteTitle, ink::INK);
                inline::show_line_with_tooltips(ui, world, &text.title, title);
                ui.add_space(dialog::TITLE_GAP);
                let said = text
                    .body
                    .iter()
                    .map(|line| {
                        let look = TextLook::role(TextRole::Body, ink);
                        inline::show_line_with_tooltips(ui, world, line, look)
                    })
                    .last();
                let slid = text.amount.as_ref().map(|amount| {
                    ui.add_space(8.0);
                    ui.spacing_mut().slider_width = ui.available_width();
                    let mut chosen = amount.chosen;
                    let slider = controls::count_slider(ui, &mut chosen, amount.amounts.clone());
                    let slider = match &said {
                        Some(said) => slider.labelled_by(said.id),
                        None => slider,
                    };
                    (slider, chosen)
                });
                let answers = ui.min_rect().top() + NOTE.y - padding - controls::CONTROL_HEIGHT;
                ui.add_space((answers - ui.cursor().top()).max(12.0));
                let row = Vec2::new(ui.available_width(), controls::CONTROL_HEIGHT);
                let across = egui::Layout::right_to_left(egui::Align::Center);
                ui.allocate_ui_with_layout(row, across, |ui| {
                    let default_pressed =
                        default_key.is_some_and(|key| ui.input_mut(|i| i.consume_shortcut(&key)));
                    let (confirm, cancel, first) = match (slid, &text.confirm) {
                        (Some((slider, chosen)), Some(confirming)) => {
                            if let Some(Asked::Amount { chosen: held, .. }) =
                                pending.as_mut().map(|pending| &mut pending.asked)
                            {
                                *held = chosen;
                            }
                            let confirm = controls::primary(ui, confirming.as_str());
                            let cancel = controls::button(ui, text.cancel.as_str());
                            if default_pressed {
                                decided = Some(true);
                            }
                            (Some(confirm), cancel, slider)
                        }
                        (_, Some(confirming)) => {
                            let keep = controls::primary(ui, text.cancel.as_str());
                            let confirm = controls::danger(ui, confirming.as_str());
                            if default_pressed {
                                decided = Some(false);
                            }
                            (Some(confirm), keep.clone(), keep)
                        }
                        (_, None) => {
                            let dismiss = controls::primary(ui, text.cancel.as_str());
                            if default_pressed {
                                decided = Some(false);
                            }
                            (None, dismiss.clone(), dismiss)
                        }
                    };
                    if let Some(pending) = pending.as_mut()
                        && !pending.focused
                    {
                        first.request_focus();
                        pending.focused = true;
                    }
                    if confirm.is_some_and(|confirm| confirm.clicked()) {
                        decided = Some(true);
                    }
                    if cancel.clicked() {
                        decided = Some(false);
                    }
                });
            });
        });
    let Some(pending) = pending else {
        return;
    };
    if modal.should_close() && decided.is_none() {
        decided = Some(false);
    }
    match decided {
        Some(decided) => pending.asked.answer(world, decided),
        None => world.resource_mut::<Confirming>().0 = Some(pending),
    }
}

/// What letting go of what is carried does where the pointer is, said beside it: what the widget
/// under the pointer set this frame.
#[derive(Resource, Default)]
struct DropHint(Option<String>);

/// Says beside what is carried what letting go of it here does, for this frame.
pub(crate) fn drop_hint(world: &mut World, hint: String) {
    world.resource_mut::<DropHint>().0 = Some(hint);
}

/// The icon of what is carried, under the pointer, and what letting go of it there does.
fn draw_carried(world: &mut World) {
    let hint = world.resource_mut::<DropHint>().0.take();
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(carried) = drag::carried::<Carried>(&gui) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let carried = match carried {
        Carried::Skill(skill) => CarriedThing::Skill(skill),
        Carried::BagSlot(slot) => {
            let Some(stack) = item::my_bag(world).and_then(|inventory| inventory.at(slot as usize))
            else {
                return;
            };
            CarriedThing::Item(stack.item)
        }
        Carried::Worn(slot) => {
            let Some(item) = session::me(world).and_then(|me| equipment::in_slot(world, me, &slot))
            else {
                return;
            };
            CarriedThing::Item(item)
        }
        Carried::BarEntry { entry, .. } => match entry {
            HotbarEntry::Skill(skill) => CarriedThing::Skill(skill),
            HotbarEntry::Item(item) => CarriedThing::Item(item),
            HotbarEntry::Empty => return,
        },
        Carried::Moved { stack, .. } => CarriedThing::Item(stack.item),
    };
    let (icon, rarity) = match carried {
        CarriedThing::Skill(skill) => (&skill.get(&content).icon, None),
        CarriedThing::Item(item) => {
            let def = item.get(&content);
            (&def.icon, Some(SlotRarity::of(&content, def.rarity)))
        }
    };
    let Some(image) = gui::image(world, icon) else {
        return;
    };
    let pip = rarity
        .and_then(|rarity| rarity.icon)
        .and_then(|icon| gui::image(world, icon));
    float::at_pointer(
        &gui,
        Id::new("carried"),
        Band::Pointer.order(),
        -Vec2::splat(CARRIED / 2.0),
        |ui| {
            let response = ui.add(image.widget(Vec2::splat(CARRIED)));
            if let Some(rarity) = rarity {
                paint::gradient_edge(
                    ui.painter(),
                    response.rect,
                    controls::SLOT_EDGE,
                    rarity.colors(),
                );
            }
            if let Some(pip) = pip {
                controls::slot_pip(ui.painter(), response.rect, pip.texture, Color32::WHITE);
            }
            if let Some(hint) = &hint {
                let galley = ui.painter().layout_no_wrap(
                    hint.clone(),
                    TextRole::Small.font_id(),
                    Color32::WHITE,
                );
                let at = response.rect.right_center() + Vec2::new(6.0, -galley.size().y / 2.0);
                ui::dusk::lettering(
                    ui.painter(),
                    at,
                    galley,
                    ui::dusk::EDGED,
                    egui::emath::TSTransform::IDENTITY,
                );
            }
        },
    );
}

enum CarriedThing {
    Skill(SkillId),
    Item(ItemId),
}

fn sized(ui: &mut Ui, image: Option<gui::GuiImage>, size: f32) -> egui::Response {
    match image {
        Some(image) => ui.add(image.widget(Vec2::splat(size))),
        None => ui.allocate_exact_size(Vec2::splat(size), Sense::hover()).1,
    }
}

messages! {
    /// What the game says on the parts of the screen around play.
    pub struct HudMessages {
        /// The button that confirms an amount the player picked with a slider.
        confirm,
        /// What a screen reader names the menu window's title bar, which the player drags to move it.
        menu,
        /// What the key that steps out of the menu does, as the menu's key hints say it.
        closes,
        /// The title of the menu's tab for the player's character.
        tab_character,
        /// The title of the menu's tab for skills.
        tab_skills,
        /// The title of the menu's tab for quests.
        tab_quests,
        /// The title of the menu's tab for the map.
        tab_map,
        /// The title of the menu's tab for what has happened to the player.
        tab_history,
        /// The title of the menu's tab for friends, parties and societies.
        tab_social,
        /// The title of the menu's tab for settings.
        tab_settings,
        /// What a menu tab's badge shows of how many are waiting, `{count}` being the number.
        badge_count(count: Number),
        /// What a menu tab's badge shows when more are waiting than it counts, `{most}` being the most it counts.
        badge_over(most: Number),
        /// What a screen reader says of the day clock when no time of day is named: the hour `{hour}`.
        clock_hour(hour: Number),
        /// What a screen reader says of the day clock: the hour `{hour}` and the name of the time of day `{time}`.
        clock_time(hour: Number, time: Text),
    }
}
