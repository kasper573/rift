//! Resources as a client shows them: each standing as the prop it looks like under a plate saying
//! what is left of it and who works it, a bar under every worker filling once an extraction, and
//! the pointer over one answering whether a click would work it and why not.

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::egui;
use ui::face::{self, TextRole};
use ui::{float, ink, paint, slip};

use super::{Left, Resource, ResourceId, StopGathering, Working, workers};
use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::math::Pos;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::{ground_height, window_point, world_point};
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::core::time;
use crate::core::units::{Meters, Seconds};
use crate::systems::actor::Hitbox;
use crate::systems::actor::plate::PlateLabel;
use crate::systems::equipment::{self, EquipmentPlace};
use crate::systems::hud;
use crate::systems::input::claims::{self, ClickClaim};
use crate::systems::input::map::{InputAction, input};
use crate::systems::item::{ExchangeRefusal, Inventory, ItemId, ToolTypeId};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::stat::{self, StatKind};

const BAR: egui::Vec2 = egui::Vec2::new(56.0, 5.0);
const BAR_DROP: f32 = 10.0;
const TIP_FROM_POINTER: egui::Vec2 = egui::Vec2::new(18.0, 18.0);
const TIP_WIDTH: f32 = 240.0;
const HINT_GAP: f32 = 8.0;

pub struct ResourceRenderPlugin;

impl Plugin for ResourceRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(dress)
            .add_systems(
                Update,
                stand
                    .in_set(WorldViewSystems)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (draw_bars, draw_outlook, draw_hint)
                    .in_set(Band::World)
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

/// How working a resource looks to a character before it clicks: what is left, who works it, and
/// what stands in the way.
#[derive(Clone, Debug, PartialEq)]
pub struct Outlook {
    pub def: ResourceId,
    pub left: u32,
    pub working: u32,
    pub room: u32,
    pub tool: ToolOutlook,
    /// How long one extraction would take the character.
    pub takes: Seconds,
    /// Why the bag could not take the most one extraction can drop.
    pub bag: Option<ExchangeRefusal>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToolOutlook {
    ByHand,
    Worn(ItemId),
    Lacking {
        kind: ToolTypeId,
        holds: Option<ItemId>,
    },
}

impl Outlook {
    pub fn full(&self) -> bool {
        self.working >= self.room
    }

    pub fn workable(&self) -> bool {
        !self.full() && self.bag.is_none() && !matches!(self.tool, ToolOutlook::Lacking { .. })
    }
}

/// How working `resource` looks to `character`, while it stands.
pub fn outlook(world: &mut World, character: Entity, resource: Entity) -> Option<Outlook> {
    let content = world.resource::<Content>().clone();
    let def = world.get::<Resource>(resource)?.def;
    let left = world.get::<Left>(resource)?.0;
    let row = def.get(&content);
    let mine = world
        .get::<Working>(character)
        .is_some_and(|working| working.resource == resource);
    let working = workers(world, resource) - u32::from(mine);
    let holds = equipment::in_slot(world, character, &EquipmentPlace::Tool);
    let tool = match row.extraction.tool() {
        None => ToolOutlook::ByHand,
        Some(kind) => {
            let fits = |item: &ItemId| {
                item.get(&content).wear().and_then(|wear| wear.tool_type()) == Some(kind)
            };
            match holds.filter(fits) {
                Some(worn) => ToolOutlook::Worn(worn),
                None => ToolOutlook::Lacking { kind, holds },
            }
        }
    };
    let speed = stat::effective(world, character, StatKind::GatherSpeed);
    let rate = stat::effective(world, character, StatKind::GatherDropRate);
    let bag = world
        .get::<Inventory>(character)
        .cloned()
        .unwrap_or_else(Inventory::empty)
        .exchange(&content, &[], &row.most_at(rate))
        .err();
    Some(Outlook {
        def,
        left,
        working,
        room: row.max_occupants,
        tool,
        takes: row.takes_at(speed),
        bag,
    })
}

/// The resource the pointer picks, if it is on one.
pub fn pointed_at(world: &mut World, pointed: &crate::core::render::Pointed) -> Option<Entity> {
    crate::systems::actor::target_at(world, pointed, None, |world, entity| {
        world.get::<Resource>(entity).is_some()
    })
}

/// Ends the work the player's character has under way, if any.
pub fn stop(world: &mut World) {
    let working = session::me(world).is_some_and(|me| world.get::<Working>(me).is_some());
    if working {
        world.write_message(StopGathering);
    }
}

/// How far through the extraction under way a worker is, from none to all of it.
pub fn progress(world: &World, working: &Working) -> Option<f32> {
    let now = time::server_now(world)?;
    let whole = working.until.since(working.since).0;
    let done = now.since(working.since).0;
    Some(if whole > 0.0 {
        (done / whole).clamp(0.0, 1.0)
    } else {
        1.0
    })
}

/// The plate over every standing resource: its name, what is left of it and who works it.
pub fn plates(world: &mut World) -> Vec<(Entity, PlateLabel, Pos<Meters>, Hitbox)> {
    let content = world.resource::<Content>().clone();
    let standing: Vec<(Entity, ResourceId, u32, Pos<Meters>, Hitbox)> = world
        .query::<(Entity, &Resource, &Left, &Position, &Hitbox)>()
        .iter(world)
        .map(|(entity, resource, left, at, hitbox)| (entity, resource.def, left.0, at.pos, *hitbox))
        .collect();
    standing
        .into_iter()
        .map(|(entity, def, left, at, hitbox)| {
            let row = def.get(&content);
            let working = workers(world, entity);
            let label = PlateLabel::resource(&row.name, left, working, row.max_occupants);
            (entity, label, at, hitbox)
        })
        .collect()
}

/// What a plate says under a resource's name.
pub fn plate_line(left: u32, working: u32, room: u32) -> String {
    match working {
        0 => format!("{left} left"),
        _ => format!("{left} left · {working} of {room} working"),
    }
}

fn dress(
    add: On<Add<Resource>>,
    resources: Query<&Resource>,
    content: Res<Content>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok(resource) = resources.get(add.entity) else {
        return;
    };
    let look = resource.def.get(&content).look.get(&content);
    let scene = crate::core::assets::model_scene(&assets, look.model.path());
    commands.entity(add.entity).insert((
        WorldAssetRoot(scene),
        Transform::default(),
        Visibility::default(),
    ));
}

fn stand(world: &mut World) {
    let standing: Vec<(Entity, Pos<Meters>)> = world
        .query_filtered::<(Entity, &Position), With<Resource>>()
        .iter(world)
        .map(|(entity, position)| (entity, position.pos))
        .collect();
    for (entity, at) in standing {
        let at = world_point(at, ground_height(world, at));
        if let Some(mut transform) = world.get_mut::<Transform>(entity)
            && transform.translation != at
        {
            transform.translation = at;
        }
    }
}

fn draw_bars(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let workers: Vec<(Entity, Pos<Meters>, Working)> = world
        .query::<(Entity, &Position, Option<&RenderPosition>, &Working)>()
        .iter(world)
        .map(|(entity, at, drawn, working)| {
            (entity, drawn.map_or(at.pos, |drawn| drawn.0), *working)
        })
        .collect();
    for (worker, at, working) in workers {
        let Some(share) = progress(world, &working) else {
            continue;
        };
        let Some(feet) = window_point(world, at, Meters(0.0)) else {
            continue;
        };
        gui::world_area(world, ("work bar", worker))
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(feet.x, feet.y + BAR_DROP))
            .show(&gui, |ui| paint::progress_line(ui, BAR, share, ink::GOLD));
    }
}

fn draw_outlook(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(me) = session::me(world) else {
        return;
    };
    let ClickClaim::Gather(resource) = claims::claim(world) else {
        return;
    };
    let Some(outlook) = outlook(world, me, resource) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let world: &World = world;
    float::at_pointer(
        &gui,
        egui::Id::new("resource outlook"),
        Band::Pointer.order(),
        TIP_FROM_POINTER,
        |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(TIP_WIDTH);
                ui.spacing_mut().item_spacing.y = 2.0;
                let line = |ui: &mut egui::Ui, text: String, color: egui::Color32| {
                    ui.label(
                        egui::RichText::new(text)
                            .size(TextRole::Tooltip.size())
                            .color(color),
                    );
                };
                ui.label(
                    face::bold(outlook.def.get(&content).name.as_str())
                        .size(TextRole::Tooltip.size()),
                );
                for (text, fine) in outlook_lines(&content, &outlook) {
                    line(ui, text, if fine { ink::INK2 } else { ink::RUBRIC });
                }
                if outlook.workable() {
                    let hint = StyledLine::new(vec![
                        input(InputAction::Interact),
                        TextPiece::text(" to work it"),
                    ]);
                    let look = TextLook::new(TextRole::Small.size(), ink::INK3);
                    text::show(ui, world, &hint, look, None);
                }
            });
        },
    );
}

/// What the pointer over a resource says of it, line by line, each fine or in the way.
pub fn outlook_lines(content: &Content, outlook: &Outlook) -> Vec<(String, bool)> {
    let mut lines = Vec::new();
    if outlook.full() {
        lines.push((format!("{} left", outlook.left), true));
        lines.push((
            format!(
                "{} · {} of {} working",
                content
                    .built::<crate::systems::GameSettings>()
                    .words
                    .no_room,
                outlook.working,
                outlook.room
            ),
            false,
        ));
    } else {
        lines.push((
            format!(
                "{} left · room for {}",
                outlook.left,
                outlook.room - outlook.working
            ),
            true,
        ));
    }
    let each = format!("{} each", seconds(outlook.takes));
    match outlook.tool {
        ToolOutlook::ByHand => lines.push((each, true)),
        ToolOutlook::Worn(tool) => {
            lines.push((
                format!("{} equipped · {each}", tool.get(content).name),
                true,
            ));
        }
        ToolOutlook::Lacking { kind, holds } => {
            let words = &content.built::<crate::systems::GameSettings>().words;
            lines.push((
                words.needs_tool.replace("{tool}", &kind.get(content).name),
                false,
            ));
            lines.push((
                match holds {
                    Some(held) => format!("Your tool slot holds {}", held.get(content).name),
                    None => "Your tool slot is empty".to_owned(),
                },
                true,
            ));
        }
    }
    if let Some(refusal) = outlook.bag {
        lines.push((refusal.describe(content), false));
    }
    lines
}

fn seconds(span: Seconds) -> String {
    let tenths = (span.0 * 10.0).round() / 10.0;
    if tenths.fract() == 0.0 {
        format!("{tenths:.0} s")
    } else {
        format!("{tenths:.1} s")
    }
}

fn draw_hint(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(working) = session::me(world).and_then(|me| world.get::<Working>(me).copied()) else {
        return;
    };
    let Some(name) = world
        .get::<Resource>(working.resource)
        .map(|resource| resource.def.get(world.resource::<Content>()).name.clone())
    else {
        return;
    };
    let screen = gui.content_rect();
    let middle = screen.center().x;
    let floor = hud::drawn_rects(world)
        .into_iter()
        .filter(|rect| rect.x_range().contains(middle) && rect.center().y > screen.center().y)
        .map(|rect| rect.top())
        .fold(screen.bottom(), f32::min);
    let hint = StyledLine::new(vec![
        TextPiece::text(format!("Working {name} · move or ")),
        input(InputAction::Dismiss),
        TextPiece::text(" to stop"),
    ]);
    let world: &World = world;
    float::anchored(
        &gui,
        egui::Area::new(egui::Id::new("working hint"))
            .order(Band::Hud.order())
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(egui::pos2(middle, floor - HINT_GAP))
            .interactable(false),
        |ui| {
            slip::show(ui, |ui| {
                let look = TextLook::new(TextRole::Small.size(), ink::INK2);
                text::show(ui, world, &hint, look, None);
            });
        },
    );
}
