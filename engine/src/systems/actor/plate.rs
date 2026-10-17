use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use design_tokens::family;
use egui::emath::{GuiRounding, TSTransform};
use egui::{Align, Color32, FontId, Layout, Sense, UiBuilder};

use crate::core::content::Content;
use crate::core::gui::{self, Band, GuiImage};
use crate::core::math::Pos;
use crate::core::render::tile_to_window;
use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};
use crate::core::text::{self, TextShadow};
use crate::core::tiling::Tiles;
use crate::systems::actor::{Action, Actor, Hitbox, Name};
use crate::systems::area::AreaObject;
use crate::systems::attention;
use crate::systems::combat::Attitude;
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::npc::Npc;
use crate::systems::player::Owner;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

const HEAD_ROOM: f32 = 6.0;
const ICON: f32 = 22.0;
const ICON_GAP: f32 = 2.0;
const TEXT_SIZE: f32 = 12.0;
const HOSTILE: Color32 = Color32::from_rgb(233, 117, 120);
const PLAYER: Color32 = Color32::from_rgb(104, 168, 235);
const SHADOW: TextShadow = TextShadow {
    offset: egui::vec2(1.0, 1.0),
    color: Color32::BLACK,
};

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
            tile_to_window(world, head(at, world.get::<Hitbox>(actor)?))
        }
    }
}

#[derive(Resource, Default)]
struct Plates(HashMap<Entity, egui::Rect>);

struct Label {
    name: Option<String>,
    role: Option<String>,
    color: Color32,
}

struct Plate {
    actor: Entity,
    label: Label,
    icons: Vec<Option<GuiImage>>,
    rect: egui::Rect,
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
    let labelled: Vec<(Entity, Label, Pos<Tiles>, Hitbox)> = world
        .query_filtered::<(
            Entity,
            Option<&Name>,
            Option<&Npc>,
            Option<&Owner>,
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
            |(actor, name, npc, owner, attitude, object, rendered, at, hitbox, _)| {
                let label = label(&content, name, npc, owner, attitude, object)?;
                let at = rendered.map_or(at.pos, |rendered| rendered.0);
                Some((actor, label, at, hitbox.clone()))
            },
        )
        .collect();
    let plates: Vec<Plate> = labelled
        .into_iter()
        .filter_map(|(actor, label, at, hitbox)| {
            let top = tile_to_window(world, head(at, &hitbox))?;
            let icons: Vec<Option<GuiImage>> = attention::plate_marks(world, viewer, actor)
                .into_iter()
                .map(|mark| gui::image(world, &mark.get(&content).icon))
                .collect();
            if label.name.is_none() && label.role.is_none() && icons.is_empty() {
                return None;
            }
            let size = plate_size(&gui, &label, &icons);
            let rect = egui::Rect::from_min_size(
                egui::pos2(top.x - size.x / 2.0, top.y - HEAD_ROOM - size.y)
                    .round_to_pixels(gui.pixels_per_point()),
                size,
            );
            Some(Plate {
                actor,
                label,
                icons,
                rect,
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
    if !plate.icons.is_empty() {
        column.horizontal(|ui| {
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
        text::show_shadowed(
            &mut column,
            name,
            name_font(),
            plate.label.color,
            SHADOW,
            TSTransform::IDENTITY,
        );
    }
    if let Some(role) = plate.label.role.as_deref() {
        text::show_shadowed(
            &mut column,
            &role_text(role),
            role_font(),
            Color32::WHITE.gamma_multiply(0.7),
            SHADOW,
            TSTransform::IDENTITY,
        );
    }
}

fn plate_size(gui: &egui::Context, label: &Label, icons: &[Option<GuiImage>]) -> egui::Vec2 {
    let text = |text: String, font: FontId| {
        gui.fonts_mut(|fonts| fonts.layout_no_wrap(text, font, Color32::WHITE))
            .size()
    };
    let icons = match icons.len() {
        0 => egui::Vec2::ZERO,
        count => egui::vec2(count as f32 * ICON + (count - 1) as f32 * ICON_GAP, ICON),
    };
    [
        Some(icons),
        label.name.clone().map(|name| text(name, name_font())),
        label
            .role
            .as_deref()
            .map(|role| text(role_text(role), role_font())),
    ]
    .into_iter()
    .flatten()
    .fold(egui::Vec2::ZERO, |size, part| {
        egui::vec2(size.x.max(part.x), size.y + part.y)
    })
}

fn label(
    content: &Content,
    name: Option<&Name>,
    npc: Option<&Npc>,
    owner: Option<&Owner>,
    attitude: Option<&Attitude>,
    object: bool,
) -> Option<Label> {
    if object {
        return Some(Label {
            name: None,
            role: None,
            color: Color32::WHITE,
        });
    }
    match (npc, owner) {
        (Some(npc), _) => {
            let def = npc.def.get(content);
            def.nameplate().map(|role| Label {
                name: Some(def.name.to_owned()),
                role: Some(role.to_owned()),
                color: match attitude {
                    Some(Attitude::Hostile) => HOSTILE,
                    _ => Color32::WHITE,
                },
            })
        }
        (None, Some(_)) => Some(Label {
            name: Some(name?.name.clone()),
            role: None,
            color: PLAYER,
        }),
        (None, None) => None,
    }
}

fn head(at: Pos<Tiles>, hitbox: &Hitbox) -> Pos<Tiles> {
    Pos::new(at.x, at.y + 0.5 - hitbox.size.height)
}

fn role_text(role: &str) -> String {
    format!("<{role}>")
}

fn name_font() -> FontId {
    FontId::new(TEXT_SIZE, family::prose_bold())
}

fn role_font() -> FontId {
    FontId::new(TEXT_SIZE, family::prose())
}
