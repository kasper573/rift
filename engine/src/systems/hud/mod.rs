mod card;
pub mod day_clock;
pub mod menu;
mod panel;

pub(crate) use card::{
    Pinned, draw_pinned, forget_pinned, pin, pinned, pinned_rect, raise_pinned, unpin,
};
pub(crate) use panel::{SizedPiece, grab_sized, grab_spot, sheet_pose, sized_rect, widget};

use std::collections::HashSet;
use ui::dialog;
use ui::drag;
use ui::egui;
use ui::float;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::egui::{Id, LayerId, Pos2, Rect, Sense, Ui, Vec2};
use ui::face::{self, TextRole};
use ui::{controls, ink, sheet};

use crate::core::content::Content;
use crate::core::content::ImageRef;
use crate::core::gui::{self, Band, ImageSource, WidgetRects};
use crate::core::text::{self, StyledLine, TextLook};
use crate::systems::area_map::minimap;
use crate::systems::input::map::{InputAction, InputMap};
use crate::systems::item::Inventory;
use crate::systems::player::{self, session};
use crate::systems::screen::ClientScreen;
use crate::systems::skill::{HotbarEntry, HotbarSlot, SkillId};
use crate::systems::{equipment, item, quest, skill};

pub(crate) const TOOLTIP_WIDTH: f32 = 260.0;
pub(crate) const MARGIN: f32 = 8.0;
const CARRIED: f32 = 40.0;
const CONFIRM: &str = "confirm";
const NOTE: Vec2 = Vec2::new(400.0, 170.0);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        menu::plugin(app);
        app.init_resource::<panel::Arranging>()
            .init_resource::<panel::SheetClocks>()
            .init_resource::<WindowsDrawn>()
            .init_resource::<Confirming>()
            .init_resource::<Lifting>()
            .add_systems(
                EguiPrimaryContextPass,
                (
                    (forget_drawn, panel::arrange).before(Band::Stage),
                    draw_hud
                        .in_set(Band::Hud)
                        .run_if(in_state(ClientScreen::Area)),
                    draw_confirm.in_set(Band::Modal),
                    draw_carried
                        .in_set(Band::Pointer)
                        .run_if(in_state(ClientScreen::Area)),
                ),
            );
    }
}

pub trait HudWidget: Send + Sync {
    fn fallback(&self, world: &World, screen: Rect) -> Pos2;
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
    pub title: String,
    pub body: Vec<String>,
    pub confirm: String,
    pub cancel: String,
    pub answer: Answer,
}

pub(crate) type Answer = Box<dyn FnOnce(&mut World, bool) + Send + Sync>;

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
        confirm,
        focused: false,
    });
}

pub(crate) fn confirming(world: &World) -> bool {
    world.resource::<Confirming>().0.is_some()
}

pub(crate) fn dismiss_confirm(world: &mut World) -> bool {
    match world.resource_mut::<Confirming>().0.take() {
        Some(pending) => {
            (pending.confirm.answer)(world, false);
            true
        }
        None => false,
    }
}

pub(crate) fn is_piece(name: &str) -> bool {
    WIDGETS.iter().any(|&(piece, _)| piece == name)
}

pub(crate) fn keeps_place(name: &str) -> bool {
    is_piece(name) || [item::card::WINDOW_KEY, skill::card::WINDOW_KEY].contains(&name)
}

pub(crate) fn window_layer(key: &'static str) -> LayerId {
    panel::layer(key)
}

pub(crate) fn confirm_layer() -> LayerId {
    LayerId::new(Band::Modal.order(), Id::new(CONFIRM))
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
    lines: &[String],
    hint: Option<&StyledLine>,
) {
    float::tooltip(response, |ui| {
        ui.set_max_width(TOOLTIP_WIDTH);
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(face::bold(title).size(TextRole::Tooltip.size()));
        for line in lines {
            ui.label(
                egui::RichText::new(line)
                    .size(TextRole::Tooltip.size())
                    .color(ink::INK2),
            );
        }
        if let Some(hint) = hint {
            let look = TextLook::new(TextRole::Small.size(), ink::INK3);
            text::show(ui, world, hint, look, None);
        }
    });
}

pub(crate) fn file_icon(ui: &mut Ui, world: &mut World, file: &str, size: f32) -> egui::Response {
    sized(ui, gui::image_file(world, file), size)
}

static WIDGETS: &[(&str, &dyn HudWidget)] = &[
    (minimap::MINIMAP, &minimap::MinimapWidget),
    (
        quest::tracker::QUEST_TRACKER,
        &quest::tracker::QuestTrackerWidget,
    ),
    (player::bar::PLAYER_BAR, &player::bar::PlayerBarWidget),
];

#[derive(Resource, Default)]
struct WindowsDrawn {
    last: HashSet<&'static str>,
    now: HashSet<&'static str>,
}

#[derive(Resource, Default)]
struct Confirming(Option<Pending>);

/// What the confirm note says, kept while the note lifts off after it is answered.
#[derive(Resource, Default)]
struct Lifting(Option<NoteText>);

#[derive(Clone)]
struct NoteText {
    title: String,
    body: Vec<String>,
    confirm: String,
    cancel: String,
}

impl NoteText {
    fn of(confirm: &Confirm) -> NoteText {
        NoteText {
            title: confirm.title.clone(),
            body: confirm.body.clone(),
            confirm: confirm.confirm.clone(),
            cancel: confirm.cancel.clone(),
        }
    }
}

struct Pending {
    confirm: Confirm,
    focused: bool,
}

/// The menu and the cards that are up, as they are stacked, the one in front first.
pub(crate) fn sheets_front_first(world: &World) -> Vec<&'static str> {
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
        let fallback = shown.fallback(world, screen);
        let past = widget(world, &gui, name, fallback, |ui, world| {
            shown.show(ui, world)
        });
        if let Some(past) = past
            && let Some(drawn) = world.resource_mut::<WidgetRects>().0.get_mut(name)
        {
            *drawn = drawn.union(past);
        }
    }
    menu::draw(world, &gui);
}

fn draw_confirm(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let pending = world.resource_mut::<Confirming>().0.take();
    let shown_text = pending
        .as_ref()
        .map(|pending| NoteText::of(&pending.confirm));
    if let Some(text) = &shown_text {
        world.resource_mut::<Lifting>().0 = Some(text.clone());
    }
    let Some(pose) = panel::sheet_pose(world, &gui, CONFIRM, pending.is_some()) else {
        world.resource_mut::<Lifting>().0 = None;
        return;
    };
    let Some(text) = shown_text.or_else(|| world.resource::<Lifting>().0.clone()) else {
        return;
    };
    let keep_key = world
        .resource::<InputMap>()
        .shortcut(InputAction::TakeDefault);
    let mut pending = pending;
    let mut decided = None;
    let screen = gui.content_rect();
    let rest = Rect::from_center_size(screen.center(), NOTE);
    gui.set_transform_layer(confirm_layer(), pose.transform(&gui, rest, rest.min));
    let modal = egui::Modal::new(Id::new(CONFIRM))
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
                ui.set_min_height(NOTE.y - padding);
                dialog::title(ui, &text.title);
                for line in &text.body {
                    ui.add(egui::Label::new(face::body(line)).wrap());
                }
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
                    let keep = controls::primary(ui, text.cancel.as_str());
                    if let Some(pending) = pending.as_mut()
                        && !pending.focused
                    {
                        keep.request_focus();
                        pending.focused = true;
                    }
                    if controls::danger(ui, text.confirm.as_str()).clicked() {
                        decided = Some(true);
                    }
                    let keep_pressed =
                        keep_key.is_some_and(|key| ui.input_mut(|i| i.consume_shortcut(&key)));
                    if keep.clicked() || keep_pressed {
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
        Some(decided) => (pending.confirm.answer)(world, decided),
        None => world.resource_mut::<Confirming>().0 = Some(pending),
    }
}

/// The icon of what is carried, under the pointer.
fn draw_carried(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(carried) = drag::carried::<Carried>(&gui) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let icon = match carried {
        Carried::Skill(skill) => skill.get(&content).icon.clone(),
        Carried::BagSlot(slot) => {
            let Some(stack) = session::my_character(world)
                .and_then(|me| me.get::<Inventory>())
                .and_then(|inventory| inventory.slots.get(slot as usize).copied())
            else {
                return;
            };
            stack.item.get(&content).icon.clone()
        }
        Carried::Worn(slot) => {
            let Some(item) = session::me(world).and_then(|me| equipment::in_slot(world, me, &slot))
            else {
                return;
            };
            item.get(&content).icon.clone()
        }
        Carried::BarEntry { entry, .. } => match entry {
            HotbarEntry::Skill(skill) => skill.get(&content).icon.clone(),
            HotbarEntry::Item(item) => item.get(&content).icon.clone(),
            HotbarEntry::Empty => return,
        },
    };
    let Some(image) = gui::image(world, &icon) else {
        return;
    };
    float::at_pointer(
        &gui,
        Id::new("carried"),
        Band::Pointer.order(),
        -Vec2::splat(CARRIED / 2.0),
        |ui| ui.add(image.widget(Vec2::splat(CARRIED))),
    );
}

fn sized(ui: &mut Ui, image: Option<gui::GuiImage>, size: f32) -> egui::Response {
    match image {
        Some(image) => ui.add(image.widget(Vec2::splat(size))),
        None => ui.allocate_exact_size(Vec2::splat(size), Sense::hover()).1,
    }
}
