use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use design_tokens::{TextRole, color, widgets};
use egui::{Color32, Pos2, Rect, Response, RichText, Ui, Vec2};

use crate::core::content::Content;
use crate::core::gui::{self, Band, GuiContext};
use crate::core::text::{self, StyledLine, StyledSpan, TextLook, TextPiece, TextVoice};
use crate::systems::hud::{self, Sizing, WindowFrame};
use crate::systems::input::map::{self, InputAction, input};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::{ItemDef, ItemFlag, ItemId, ItemKind};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::script::condition;
use crate::systems::skill::{self, window::CarriedSkill};
use crate::systems::stat::StatKind;
use crate::systems::status;

const WINDOW_KEY: &str = "Item";
const WINDOW_SIZE: Vec2 = Vec2::new(300.0, 360.0);
const WINDOW_TOP: f32 = 96.0;
const WINDOW_MARGIN: f32 = 72.0;
const ICON: f32 = 48.0;

pub struct ItemCardPlugin;

impl Plugin for ItemCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ItemCard>()
            .add_systems(Update, bring_to_front.run_if(in_state(ClientScreen::Area)))
            .add_systems(
                EguiPrimaryContextPass,
                draw_card
                    .in_set(Band::Hud)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), forget);
    }
}

pub fn open(world: &mut World, item: ItemId) {
    let mut card = world.resource_mut::<ItemCard>();
    let was_shown = card.wanted.replace(item);
    card.to_front = true;
    if was_shown.is_none() {
        InterfaceSound::UiOpen.play(world);
    }
}

pub fn close(world: &mut World) -> bool {
    let closed = world.resource_mut::<ItemCard>().wanted.take().is_some();
    if closed {
        InterfaceSound::UiClose.play(world);
    }
    closed
}

pub fn showing(world: &World) -> bool {
    world.resource::<ItemCard>().wanted.is_some()
}

pub fn layer() -> egui::LayerId {
    crate::systems::hud::window_layer(WINDOW_KEY)
}

pub fn shown(world: &World) -> Option<(ItemId, Rect)> {
    world.resource::<ItemCard>().drawn
}

/// `response` must sense clicks.
pub fn inspect(world: &mut World, response: Response, item: ItemId) -> Response {
    if map::clicked(world, InputAction::InspectItem, &response) {
        open(world, item);
    }
    tooltip(world, &response, item);
    response
}

pub fn tooltip(world: &World, response: &Response, item: ItemId) {
    let content = world.resource::<Content>().clone();
    let def = item.get(&content);
    let lines = [overview(def)];
    let hint = StyledLine::new(inspect_hint());
    hud::tooltip(response, world, &def.name, &lines, Some(&hint));
}

pub fn overview(def: &ItemDef) -> String {
    let category = def.category().label();
    match &def.kind {
        ItemKind::Equipment { slot, .. } => format!("{category} · {}", slot.label()),
        _ => category.to_owned(),
    }
}

pub fn inspect_hint() -> Vec<TextPiece> {
    vec![
        input(InputAction::InspectItem),
        TextPiece::text(" for more information"),
    ]
}

pub fn use_verb(def: &ItemDef) -> Option<&'static str> {
    match def.kind {
        ItemKind::Usable => Some("use"),
        ItemKind::Equipment { .. } => Some("wear"),
        ItemKind::Resource => None,
    }
}

pub fn sheet(ui: &mut Ui, world: &mut World, item: ItemId) {
    let content = world.resource::<Content>().clone();
    let def = item.get(&content);
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.horizontal(|ui| {
        hud::icon(ui, world, &def.icon, ICON);
        ui.vertical(|ui| {
            ui.label(RichText::new(def.name.as_str()).font(TextRole::Heading.font_id()));
            ui.label(
                RichText::new(overview(def))
                    .size(TextRole::Small.size())
                    .color(color::MUTED),
            );
        });
    });
    let tags = tags(def);
    if !tags.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (label, color) in tags {
                widgets::pill(ui, label, color);
            }
        });
    }
    if let Some(flavor) = def.flavor() {
        let line = StyledLine::new(vec![
            StyledSpan::plain(flavor.to_owned())
                .voice(TextVoice::Whisper)
                .into(),
        ]);
        let look = TextLook {
            size: TextRole::Prose.size(),
            color: color::TEXT,
        };
        text::show(ui, world, &line, look, None);
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        for (fact, color) in facts(world, item, def) {
            ui.label(RichText::new(fact).color(color));
        }
    });
    grants(ui, world, item);
}

const GRANT_ICON: f32 = 24.0;

fn grants(ui: &mut Ui, world: &mut World, item: ItemId) {
    let content = world.resource::<Content>().clone();
    let granted = skill::grants_of(&content, item);
    if granted.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new("Grants while worn")
            .size(TextRole::Small.size())
            .family(design_tokens::family::medium())
            .color(color::MUTED),
    );
    for &granted in granted {
        let def = granted.get(&content);
        let response = ui
            .horizontal(|ui| {
                hud::icon(ui, world, &def.icon, GRANT_ICON);
                ui.label(&def.name);
                ui.label(
                    RichText::new(def.target.describe())
                        .size(TextRole::Small.size())
                        .color(color::MUTED),
                );
                if def.basic_attack {
                    widgets::pill(ui, skill::card::BASIC_ATTACK_MARK, color::WARN);
                }
            })
            .response
            .interact(egui::Sense::click_and_drag());
        if map::drag_started(world, InputAction::CarryItem, &response) {
            egui::DragAndDrop::set_payload(ui.ctx(), CarriedSkill(granted));
        } else if map::clicked(world, InputAction::PinToHotbar, &response) {
            skill::bar::pin(world, skill::HotbarEntry::Skill(granted));
        }
        let hint = StyledLine::new(vec![
            TextPiece::text("Drag it to a slot · "),
            input(InputAction::PinToHotbar),
            TextPiece::text(" puts it on the hotbar"),
        ]);
        skill::card::tooltip(
            &response,
            world,
            &skill::card::of(world, granted),
            Some(&hint),
        );
    }
    if granted.iter().any(|skill| skill.get(&content).basic_attack) {
        ui.label(
            RichText::new(
                "Dropping the item on the basic attack slot picks its first basic attack",
            )
            .size(TextRole::Small.size())
            .color(color::FAINT),
        );
    }
}

#[derive(Resource, Default)]
struct ItemCard {
    wanted: Option<ItemId>,
    to_front: bool,
    drawn: Option<(ItemId, Rect)>,
}

fn forget(mut card: ResMut<ItemCard>) {
    *card = ItemCard::default();
}

// A press brings the area it lands on to the front as its pass ends, and areas brought to the front
// in the same pass keep their order. So the card comes to the front a pass after it was asked for,
// to end up above the window it was asked for from.
fn bring_to_front(mut card: ResMut<ItemCard>, gui: Option<Res<GuiContext>>) {
    if !card.to_front {
        return;
    }
    card.to_front = false;
    if let Some(gui) = gui
        && card.wanted.is_some()
    {
        gui.0.move_to_top(layer());
    }
}

fn draw_card(world: &mut World) {
    let Some(item) = world.resource::<ItemCard>().wanted else {
        world.resource_mut::<ItemCard>().drawn = None;
        return;
    };
    let Some(gui) = gui::context(world) else {
        return;
    };
    let screen = gui.content_rect();
    let start = Rect::from_min_size(
        Pos2::new(
            (screen.right() - WINDOW_SIZE.x - WINDOW_MARGIN).max(8.0),
            screen.top() + WINDOW_TOP,
        ),
        WINDOW_SIZE,
    );
    let mut open = true;
    let frame = WindowFrame {
        key: WINDOW_KEY,
        title: None,
        start,
        sizing: Sizing::FitsContent,
        body_limit: None,
    };
    let shown = hud::window(world, &gui, frame, &mut open, |ui, world| {
        egui::ScrollArea::vertical()
            .max_height(WINDOW_SIZE.y)
            .auto_shrink([false, true])
            .show(ui, |ui| sheet(ui, world, item));
    });
    world.resource_mut::<ItemCard>().drawn = Some((item, shown.response.rect));
    if !open {
        close(world);
    }
}

fn tags(def: &ItemDef) -> Vec<(&'static str, Color32)> {
    [
        (ItemFlag::Quest, "Quest item", color::WARN),
        (ItemFlag::Bound, "Bound", color::DANGER),
    ]
    .into_iter()
    .filter(|(flag, ..)| def.has(*flag))
    .map(|(_, label, color)| (label, color))
    .collect()
}

fn facts(world: &World, item: ItemId, def: &ItemDef) -> Vec<(String, Color32)> {
    let content = world.resource::<Content>();
    let plain = |fact: String| (fact, color::TEXT);
    let mut facts = Vec::new();
    let boosts = status::boosts_of(content, item);
    match &def.kind {
        ItemKind::Equipment { slot, requires, .. } => {
            facts.push(plain(format!(
                "Worn as your {}",
                slot.label().to_lowercase()
            )));
            facts.extend(boosts.iter().map(|boost| plain(modifier(*boost))));
            let viewer = session::me(world);
            facts.extend(requires.iter().map(|requirement| {
                let color = match viewer.map(|viewer| condition::holds(world, viewer, requirement))
                {
                    Some(true) => color::OK,
                    Some(false) => color::DANGER,
                    None => color::TEXT,
                };
                (
                    format!("Requires {}", condition::describe(content, requirement)),
                    color,
                )
            }));
        }
        ItemKind::Usable => facts.push(plain("Can be used".to_owned())),
        ItemKind::Resource => facts.extend(
            boosts
                .iter()
                .map(|boost| plain(format!("{} while carried", modifier(*boost)))),
        ),
    }
    match def.stack_max() {
        1 => {}
        u32::MAX => facts.push(plain("Stacks without limit".to_owned())),
        max => facts.push(plain(format!("Stacks up to {max}"))),
    }
    if def.bound() {
        facts.push(plain("Can't be sold or dropped".to_owned()));
    }
    facts
}

fn modifier((stat, by): (StatKind, Option<f32>)) -> String {
    let name: &'static str = stat.into();
    match by {
        Some(by) => format!("{by:+} {name}"),
        None => format!("Boosts {name}"),
    }
}
