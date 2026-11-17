use std::collections::HashMap;
use ui::dusk;
use ui::egui;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::egui::emath::{GuiRounding, TSTransform};
use ui::egui::{Align, Color32, FontId, Layout, Sense, UiBuilder};
use ui::face::Face;
use ui::ink::{Ground, Prose};

use crate::core::content::Content;
use crate::core::gui::{self, Band, GuiImage};
use crate::core::math::Pos;
use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};
use crate::core::render::window_point;
use crate::core::render::world::opacity::Opacity;
use crate::core::units::Meters;
use crate::systems::GameSettings;
use crate::systems::actor::{Action, Actor, Hitbox, Name};
use crate::systems::area::AreaObject;
use crate::systems::attention;
use crate::systems::combat::Attitude;
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::npc::Npc;
use crate::systems::player::{Away, PlayedBy, session};
use crate::systems::resource;
use crate::systems::screen::ClientScreen;
use ui::paint;

const HEAD_ROOM: f32 = 6.0;
const ICON: f32 = 22.0;
const ICON_GAP: f32 = 2.0;
const TEXT_SIZE: f32 = 12.0;
const HOSTILE: Color32 = Prose::Danger.on(Ground::Dusk);

pub struct PlatePlugin;

impl Plugin for PlatePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Plates>()
            .add_systems(
                EguiPrimaryContextPass,
                draw_plates
                    .in_set(Band::World)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), clear_plates);
    }
}

pub fn crown(world: &mut World, actor: Entity) -> Option<Vec2> {
    let plate = world.resource::<Plates>().0.get(&actor).copied();
    match plate {
        Some(rect) => Some(Vec2::new(rect.center().x, rect.min.y)),
        None => {
            let at = world
                .get::<RenderPosition>(actor)
                .map(|rendered| rendered.0)
                .or_else(|| world.get::<Position>(actor).map(|position| position.pos))?;
            let lift = super::lift(world, actor, at);
            window_point(world, at, lift + world.get::<Hitbox>(actor)?.0.height)
        }
    }
}

#[derive(Resource, Default)]
struct Plates(HashMap<Entity, egui::Rect>);

/// What a plate says: a name in the colour of its wearer's standing, and under a resource's who
/// works it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateLabel {
    pub name: Option<String>,
    pub state: Option<String>,
    pub color: Color32,
}

impl PlateLabel {
    pub fn npc(name: &str, hostile: bool) -> PlateLabel {
        PlateLabel {
            name: Some(name.to_owned()),
            state: None,
            color: if hostile { HOSTILE } else { Color32::WHITE },
        }
    }

    pub fn resource(name: &str, working: u32, room: u32) -> PlateLabel {
        PlateLabel {
            name: Some(name.to_owned()),
            state: resource::render::plate_line(working, room),
            color: Color32::WHITE,
        }
    }

    /// A character's plate, under its name what is `said` of it while its player is gone.
    pub fn character(name: &str, said: Option<&str>) -> PlateLabel {
        PlateLabel {
            name: Some(name.to_owned()),
            state: said.map(str::to_owned),
            color: dusk::CHARACTER,
        }
    }
}

/// The plate `actor` wears in `world`, if it wears one.
pub fn label_of(world: &World, content: &Content, actor: Entity) -> Option<PlateLabel> {
    label(
        content,
        world.get::<Name>(actor),
        world.get::<Npc>(actor),
        world.get::<PlayedBy>(actor),
        world.get::<Away>(actor).is_some(),
        world.get::<Attitude>(actor),
        world.get::<AreaObject>(actor).is_some(),
    )
}

/// Draws a plate without marks, hanging over `top` as the game hangs them over an actor's head.
pub fn draw_label(ui: &mut egui::Ui, top: egui::Pos2, label: &PlateLabel) {
    let size = plate_size(ui.ctx(), label, &[]);
    draw_plate(
        ui,
        &Plate {
            actor: Entity::PLACEHOLDER,
            label: label.clone(),
            icons: Vec::new(),
            rect: plate_rect(ui.ctx(), top, size),
            shown: 1.0,
        },
    );
}

struct Plate {
    actor: Entity,
    label: PlateLabel,
    icons: Vec<Option<GuiImage>>,
    rect: egui::Rect,
    /// How much of the plate shows: as much as what it is over does.
    shown: f32,
}

fn clear_plates(mut plates: ResMut<Plates>) {
    plates.0.clear();
}

// One area holds every plate, so they all sit under the bubbles that float above them.
pub fn draw_plates(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let mut plates = world.resource_mut::<Plates>();
    plates.0.clear();
    if world.resource::<ActiveTransition>().phase() != ScreenTransitionPhase::Idle {
        return;
    }
    let content = world.resource::<Content>().clone();
    let viewer = session::me(world);
    let mut labelled: Vec<(Entity, PlateLabel, Pos<Meters>, Hitbox)> = world
        .query_filtered::<(
            Entity,
            Option<&Name>,
            Option<&Npc>,
            Option<&PlayedBy>,
            Has<Away>,
            Option<&Attitude>,
            Has<AreaObject>,
            Option<&RenderPosition>,
            &Position,
            &Hitbox,
            Option<&Actor>,
        ), Or<(With<Actor>, With<AreaObject>)>>()
        .iter(world)
        .filter(|(actor, ..)| viewer != Some(*actor))
        .filter(|(.., body)| body.is_none_or(|body| body.action != Action::Dead))
        .filter_map(
            |(actor, name, npc, owner, away, attitude, object, rendered, at, hitbox, _)| {
                let label = label(&content, name, npc, owner, away, attitude, object)?;
                let at = rendered.map_or(at.pos, |rendered| rendered.0);
                Some((actor, label, at, *hitbox))
            },
        )
        .collect();
    labelled.extend(resource::render::plates(world));
    let plates: Vec<Plate> = labelled
        .into_iter()
        .filter_map(|(actor, label, at, hitbox)| {
            let lift = super::lift(world, actor, at);
            let top = window_point(world, at, lift + hitbox.0.height)?;
            let icons: Vec<Option<GuiImage>> = attention::plate_marks(world, viewer, actor)
                .into_iter()
                .map(|mark| gui::image(world, &mark.get(&content).icon))
                .collect();
            if label.name.is_none() && label.state.is_none() && icons.is_empty() {
                return None;
            }
            let size = plate_size(&gui, &label, &icons);
            let rect = plate_rect(&gui, egui::pos2(top.x, top.y), size);
            Some(Plate {
                actor,
                label,
                icons,
                rect,
                shown: Opacity::of(world, actor),
            })
        })
        .collect();
    gui::world_area(world, "plates")
        .fixed_pos(egui::Pos2::ZERO)
        .show(&gui, |ui| {
            for plate in &plates {
                draw_plate(ui, plate);
            }
        });
    world.resource_mut::<Plates>().0 = plates
        .iter()
        .map(|plate| (plate.actor, plate.rect))
        .collect();
}

fn draw_plate(ui: &mut egui::Ui, plate: &Plate) {
    let mut column = ui.new_child(
        UiBuilder::new()
            .max_rect(plate.rect)
            .layout(Layout::top_down(Align::Center)),
    );
    column.spacing_mut().item_spacing = egui::Vec2::ZERO;
    column.set_opacity(plate.shown);
    if !plate.icons.is_empty() {
        let row = Layout::left_to_right(Align::Center);
        column.allocate_ui_with_layout(icons_size(plate.icons.len()), row, |ui| {
            ui.spacing_mut().item_spacing.x = ICON_GAP;
            for icon in &plate.icons {
                match icon {
                    Some(icon) => {
                        ui.add(icon.widget(egui::Vec2::splat(ICON)));
                    }
                    None => {
                        ui.allocate_exact_size(egui::Vec2::splat(ICON), Sense::hover());
                    }
                }
            }
        });
    }
    if let Some(name) = &plate.label.name {
        dusk::label(
            &mut column,
            name,
            name_font(),
            plate.label.color,
            dusk::EDGED,
            TSTransform::IDENTITY,
        );
    }
    if let Some(state) = &plate.label.state {
        dusk::label(
            &mut column,
            state,
            state_font(),
            Color32::WHITE.gamma_multiply(0.7),
            dusk::EDGED,
            TSTransform::IDENTITY,
        );
    }
}

fn plate_rect(gui: &egui::Context, top: egui::Pos2, size: egui::Vec2) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(top.x - size.x / 2.0, top.y - HEAD_ROOM - size.y)
            .round_to_pixels(gui.pixels_per_point()),
        size,
    )
}

fn plate_size(gui: &egui::Context, label: &PlateLabel, icons: &[Option<GuiImage>]) -> egui::Vec2 {
    let text = |text: String, font: FontId| paint::text_size(gui, &text, font);
    [
        Some(icons_size(icons.len())),
        label.name.clone().map(|name| text(name, name_font())),
        label.state.clone().map(|state| text(state, state_font())),
    ]
    .into_iter()
    .flatten()
    .fold(egui::Vec2::ZERO, |size, part| {
        egui::vec2(size.x.max(part.x), size.y + part.y)
    })
}

fn icons_size(count: usize) -> egui::Vec2 {
    match count {
        0 => egui::Vec2::ZERO,
        count => egui::vec2(count as f32 * ICON + (count - 1) as f32 * ICON_GAP, ICON),
    }
}

fn label(
    content: &Content,
    name: Option<&Name>,
    npc: Option<&Npc>,
    owner: Option<&PlayedBy>,
    away: bool,
    attitude: Option<&Attitude>,
    object: bool,
) -> Option<PlateLabel> {
    if object {
        return Some(PlateLabel {
            name: None,
            state: None,
            color: Color32::WHITE,
        });
    }
    match (npc, owner) {
        (Some(npc), _) => {
            let def = npc.def.get(content);
            def.wears_nameplate()
                .then(|| PlateLabel::npc(&def.name, matches!(attitude, Some(Attitude::Hostile))))
        }
        (None, Some(_)) => {
            let words = &content.built::<GameSettings>().words;
            Some(PlateLabel::character(
                &name?.name,
                away.then_some(words.away.as_str()),
            ))
        }
        (None, None) => None,
    }
}

fn name_font() -> FontId {
    Face::BookBold.id(TEXT_SIZE)
}

fn state_font() -> FontId {
    Face::Book.id(TEXT_SIZE)
}
