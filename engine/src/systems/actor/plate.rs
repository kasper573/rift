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
use crate::core::render::world::opacity::Opacity;
use crate::core::render::{self, window_point};
use crate::core::units::Meters;
use crate::systems::GameSettings;
use crate::systems::actor::{Action, Actor, Hitbox, Name};
use crate::systems::area::AreaObject;
use crate::systems::attention;
use crate::systems::combat::Attitude;
use crate::systems::messages;
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::npc::Npc;
use crate::systems::player::{Away, PlayedBy, session};
use crate::systems::resource::{self, ResourceMessages};
use crate::systems::screen::ClientScreen;
use crate::systems::settings::UserSettings;
use ui::paint;

const HEAD_ROOM: f32 = 6.0;
/// How large the most pressing mark is drawn, over the name.
const MARK: f32 = 34.0;
/// How large each further mark is drawn, after the name.
const PIP: f32 = 20.0;
const PIP_GAP: f32 = 3.0;
/// The most further marks a plate shows, so a busy townsperson never wears a wall of them.
const MOST_PIPS: usize = 3;
const TEXT_SIZE: f32 = 15.0;
/// How large a plate is at the edge of sight, beside its size at the player.
const FARTHEST: f32 = 0.8;
/// How large the plate of what the pointer is on is drawn, to stand out of a crowd.
const POINTED: f32 = 1.1;
/// The steps a plate's size moves in, so its words are laid out at a few sizes only.
const SCALE_STEP: f32 = 0.05;
/// How far below a mark its shadow falls, so its edge holds on any ground.
const MARK_DROP: f32 = 1.5;
const MARK_SHADOW: Color32 = Color32::from_black_alpha(130);
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

    pub fn resource(words: &ResourceMessages, name: &str, working: u32, room: u32) -> PlateLabel {
        PlateLabel {
            name: Some(name.to_owned()),
            state: resource::render::plate_line(words, working, room),
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
    draw_marked(ui, top, label, &[], 1.0);
}

/// Draws a plate wearing `marks`, most pressing first, `scale` times its size, hanging over
/// `top` as the game hangs them over an actor's head.
pub fn draw_marked(
    ui: &mut egui::Ui,
    top: egui::Pos2,
    label: &PlateLabel,
    marks: &[Option<GuiImage>],
    scale: f32,
) {
    let scale = stepped(scale);
    let size = plate_size(ui.ctx(), label, marks, scale);
    draw_plate(
        ui,
        &Plate {
            actor: Entity::PLACEHOLDER,
            label: label.clone(),
            marks: marks.to_vec(),
            rect: plate_rect(ui.ctx(), top, size),
            scale,
            shown: 1.0,
        },
    );
}

struct Plate {
    actor: Entity,
    label: PlateLabel,
    /// The marks it wears, most pressing first.
    marks: Vec<Option<GuiImage>>,
    rect: egui::Rect,
    /// How many times its size it is drawn.
    scale: f32,
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
    let sized = world.resource::<UserSettings>().nameplate_size().0 / 100.0;
    let sight = content.built::<GameSettings>().sight;
    let eye = viewer.and_then(|viewer| world.get::<Position>(viewer).map(|at| at.pos));
    let pointed = render::pointed(world)
        .and_then(|pointed| super::target_at(world, &pointed, viewer, |_, _| true));
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
    let mut plates: Vec<(f32, Plate)> = labelled
        .into_iter()
        .filter_map(|(actor, label, at, hitbox)| {
            let lift = super::lift(world, actor, at);
            let top = window_point(world, at, lift + hitbox.0.height)?;
            let marks: Vec<Option<GuiImage>> = attention::plate_marks(world, viewer, actor)
                .into_iter()
                .take(1 + MOST_PIPS)
                .map(|mark| gui::image(world, &mark.get(&content).icon))
                .collect();
            if label.name.is_none() && label.state.is_none() && marks.is_empty() {
                return None;
            }
            let away = eye.map_or(0.0, |eye| eye.distance_to(at));
            let far = (away / sight.0.max(f32::EPSILON)).clamp(0.0, 1.0);
            let near = 1.0 - (1.0 - FARTHEST) * far;
            let scale = stepped(sized * near * if pointed == Some(actor) { POINTED } else { 1.0 });
            let size = plate_size(&gui, &label, &marks, scale);
            let rect = plate_rect(&gui, egui::pos2(top.x, top.y), size);
            // Far plates first and the pointed one last, so the nearer is whole where two meet.
            let order = if pointed == Some(actor) {
                f32::NEG_INFINITY
            } else {
                -away
            };
            let plate = Plate {
                actor,
                label,
                marks,
                rect,
                scale,
                shown: Opacity::of(world, actor),
            };
            Some((order, plate))
        })
        .collect();
    plates.sort_by(|(a, _), (b, _)| a.total_cmp(b));
    let plates: Vec<Plate> = plates.into_iter().map(|(_, plate)| plate).collect();
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

// The most pressing mark stands over the name, the others follow it as pips.
fn draw_plate(ui: &mut egui::Ui, plate: &Plate) {
    let scale = plate.scale;
    let mut column = ui.new_child(
        UiBuilder::new()
            .max_rect(plate.rect)
            .layout(Layout::top_down(Align::Center)),
    );
    column.spacing_mut().item_spacing = egui::Vec2::ZERO;
    column.set_opacity(plate.shown);
    if let Some(mark) = plate.marks.first() {
        let (rect, _) = column.allocate_exact_size(egui::Vec2::splat(MARK * scale), Sense::hover());
        if let Some(mark) = mark {
            let painter = column.painter();
            let whole = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
            let dropped = rect.translate(egui::vec2(0.0, MARK_DROP * scale));
            painter.image(mark.texture, dropped, whole, MARK_SHADOW);
            painter.image(mark.texture, rect, whole, Color32::WHITE);
        }
    }
    let pips = pips(&plate.marks);
    let name = plate.label.name.as_ref();
    if name.is_some() || !pips.is_empty() {
        let row = Layout::left_to_right(Align::Center);
        let size = name_row_size(column.ctx(), name, pips.len(), scale);
        column.allocate_ui_with_layout(size, row, |ui| {
            ui.spacing_mut().item_spacing.x = PIP_GAP * scale;
            if let Some(name) = name {
                dusk::label(
                    ui,
                    name,
                    name_font(scale),
                    plate.label.color,
                    dusk::EDGED,
                    TSTransform::IDENTITY,
                );
            }
            for pip in pips {
                let side = egui::Vec2::splat(PIP * scale);
                match pip {
                    Some(pip) => {
                        ui.add(pip.widget(side));
                    }
                    None => {
                        ui.allocate_exact_size(side, Sense::hover());
                    }
                }
            }
        });
    }
    if let Some(state) = &plate.label.state {
        dusk::label(
            &mut column,
            state,
            state_font(scale),
            Color32::WHITE.gamma_multiply(0.7),
            dusk::EDGED,
            TSTransform::IDENTITY,
        );
    }
}

/// How much room a plate wearing `marks` takes, `scale` times its size.
pub fn size_of(
    gui: &egui::Context,
    label: &PlateLabel,
    marks: &[Option<GuiImage>],
    scale: f32,
) -> egui::Vec2 {
    plate_size(gui, label, marks, stepped(scale))
}

/// The marks after the most pressing one that a plate shows.
fn pips(marks: &[Option<GuiImage>]) -> &[Option<GuiImage>] {
    marks
        .get(1..)
        .map_or(&[], |rest| &rest[..rest.len().min(MOST_PIPS)])
}

/// `scale` to the nearest step plates are drawn at.
fn stepped(scale: f32) -> f32 {
    ((scale / SCALE_STEP).round() * SCALE_STEP).max(SCALE_STEP)
}

fn plate_rect(gui: &egui::Context, top: egui::Pos2, size: egui::Vec2) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(top.x - size.x / 2.0, top.y - HEAD_ROOM - size.y)
            .round_to_pixels(gui.pixels_per_point()),
        size,
    )
}

fn plate_size(
    gui: &egui::Context,
    label: &PlateLabel,
    marks: &[Option<GuiImage>],
    scale: f32,
) -> egui::Vec2 {
    let pips = pips(marks).len();
    let named = label.name.is_some() || pips > 0;
    [
        (!marks.is_empty()).then(|| egui::Vec2::splat(MARK * scale)),
        named.then(|| name_row_size(gui, label.name.as_ref(), pips, scale)),
        label
            .state
            .as_ref()
            .map(|state| paint::text_size(gui, state, state_font(scale))),
    ]
    .into_iter()
    .flatten()
    .fold(egui::Vec2::ZERO, |size, part| {
        egui::vec2(size.x.max(part.x), size.y + part.y)
    })
}

/// The name and the pips after it, side by side.
fn name_row_size(
    gui: &egui::Context,
    name: Option<&String>,
    pips: usize,
    scale: f32,
) -> egui::Vec2 {
    let words = name.map_or(egui::Vec2::ZERO, |name| {
        paint::text_size(gui, name, name_font(scale))
    });
    let parts = usize::from(name.is_some()) + pips;
    let pip = PIP * scale;
    egui::vec2(
        words.x + pips as f32 * pip + parts.saturating_sub(1) as f32 * PIP_GAP * scale,
        words.y.max(if pips > 0 { pip } else { 0.0 }),
    )
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
            let party = &messages::of(content).party;
            Some(PlateLabel::character(
                &name?.name,
                away.then(|| party.away()),
            ))
        }
        (None, None) => None,
    }
}

fn name_font(scale: f32) -> FontId {
    Face::BookBold.id(TEXT_SIZE * scale)
}

fn state_font(scale: f32) -> FontId {
    Face::Book.id(TEXT_SIZE * scale)
}
