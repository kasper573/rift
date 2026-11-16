//! The names of items on the ground: which are named and why, the plates that name them stacked
//! over each pile, and the item a click on a plate takes.

use std::collections::BTreeSet;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2};
use ui::face::Face;
use ui::glyph::Mark;
use ui::{dusk, ink, paint};

use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::math::Pos;
use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};
use crate::core::render::world::outline::Outline;
use crate::core::render::{self, window_point};
use crate::core::time;
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::GameSettings;
use crate::systems::actor::Name;
use crate::systems::dialogue;
use crate::systems::input::map::{self, InputAction};
use crate::systems::item::rarity::{DropNaming, ItemRarityDef, RarityRank};
use crate::systems::item::render::DropAnim;
use crate::systems::item::{self, DroppedItem, Inventory, ItemId, ItemStack, Reservation};
use crate::systems::movement::{Position, position};
use crate::systems::player::{PlayedBy, session};
use crate::systems::screen::ClientScreen;
use crate::systems::settings::UserSettings;

pub struct ItemPlatePlugin;

impl Plugin for ItemPlatePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ItemNaming>()
            .init_resource::<NamingItems>()
            .init_resource::<PointedItem>()
            .init_resource::<OpenedPile>()
            .init_resource::<DrawnPlates>()
            .add_systems(Update, read_naming_key.run_if(in_state(ClientScreen::Area)))
            .add_systems(
                EguiPrimaryContextPass,
                draw_plates
                    .in_set(Band::World)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), forget_plates);
    }
}

/// Why an item on the ground is named.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Named {
    /// Its rarity names it as it lies.
    Unprompted,
    /// The player asked for the names of the items around them, wherever the pointer rests.
    ByKey,
    /// Only the pointer names it, resting on it or on its plate.
    ByPointer,
}

/// How a player has items named.
#[derive(Resource, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ItemNaming {
    /// Whether the key switches names on and off rather than showing them while it is held.
    #[serde(default)]
    pub switched: bool,
    /// Whether a rarity may name its own items as they drop.
    #[serde(default = "crate::core::content::serial::yes")]
    pub as_dropped: bool,
    /// The lowest rank the key names.
    #[serde(default)]
    pub from: RarityRank,
}

impl Default for ItemNaming {
    fn default() -> ItemNaming {
        ItemNaming {
            switched: false,
            as_dropped: true,
            from: RarityRank(0),
        }
    }
}

/// Whether the player is asking for the names of the items around them.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NamingItems(pub bool);

/// The item on the ground the pointer rests on, and why its plate shows where the pointer is on
/// that rather than on the item.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointedItem {
    pub item: Option<Entity>,
    pub plate: Option<Named>,
}

/// When an item came to lie on the ground in front of the player, by the world's clock.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Landed(pub Seconds);

/// When the pointer last rested on the names of the pile an item lies in, by the world's clock:
/// a name does not run out under the pointer, where the pile's column would shift beneath it.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Rested(Seconds);

/// The items named now and why, the highest ranks first and then the nearest, and those left
/// unnamed for want of room.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NamedItems {
    pub shown: Vec<(Entity, Named)>,
    pub folded: Vec<Entity>,
}

/// One pile's names: a line for each kind of item named in it, the highest rank on top and then
/// by name, and how many kinds lie in it unnamed.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateColumn {
    pub over: Pos<Meters>,
    pub lines: Vec<PlateLine>,
    pub folded: Vec<Entity>,
}

/// The name of every item of one kind in a pile that may be taken alike, the nearest first.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateLine {
    pub item: ItemId,
    pub items: Vec<Entity>,
    pub count: u32,
    pub why: Named,
    pub mine: bool,
    /// Whose it is, when it is reserved for another player: theirs alone, or their party's.
    pub whose: Option<String>,
}

/// Which items on the ground are named now. None is while the player talks or the screen
/// changes; an item in the air has no name yet. The key names the player's items within reach
/// from the rank they chose, the pointer names what else it rests on, whoever may take it, and a
/// rarity names its own as its file says. More than the game shows at once fold, the lowest ranks
/// and then the farthest first, and never the one the pointer rests on.
pub fn named(world: &mut World) -> NamedItems {
    let Some(me) = session::me(world) else {
        return NamedItems::default();
    };
    let changing = world
        .get_resource::<ActiveTransition>()
        .is_some_and(|transition| transition.phase() != ScreenTransitionPhase::Idle);
    let Some(eye) = position(world, me).filter(|_| !changing) else {
        return NamedItems::default();
    };
    if dialogue::in_conversation(world, me) {
        return NamedItems::default();
    }
    let content = world.resource::<Content>().clone();
    let drops = content.built::<GameSettings>().drops;
    let naming = *world.resource::<ItemNaming>();
    let asked = world.resource::<NamingItems>().0;
    let pointed = world.resource::<PointedItem>().item;
    let opened = world
        .get_resource::<OpenedPile>()
        .map(|pile| pile.0.clone())
        .unwrap_or_default();
    let account = item::DropTaker::me(world);
    let now = time::uptime(world);
    let mut wanted: Vec<(Entity, Named, RarityRank, Meters)> = world
        .query_filtered::<(
            Entity,
            &DroppedItem,
            &Position,
            Option<&Reservation>,
            Option<&Landed>,
            Option<&Rested>,
        ), Without<DropAnim>>()
        .iter(world)
        .filter_map(|(entity, dropped, at, reservation, landed, rested)| {
            let rarity = dropped.item.get(&content).rarity.get(&content);
            let away = eye.distance(at.pos);
            let near = away <= drops.names_reach;
            let mine = item::takeable(reservation, &account);
            let why = if mine && asked && near && rarity.rank >= naming.from {
                Named::ByKey
            } else if pointed == Some(entity) {
                Named::ByPointer
            } else if !mine {
                return None;
            } else if naming.as_dropped && names_itself(rarity, landed, rested, near, now).is_some()
            {
                Named::Unprompted
            } else {
                return None;
            };
            Some((entity, why, rarity.rank, away))
        })
        .collect();
    wanted.sort_by(|a, b| {
        (pointed == Some(b.0))
            .cmp(&(pointed == Some(a.0)))
            .then(b.2.cmp(&a.2))
            .then(a.3.0.total_cmp(&b.3.0))
            .then(a.0.cmp(&b.0))
    });
    let mut named = NamedItems::default();
    for (entity, why, ..) in wanted {
        if named.shown.len() < drops.names_shown as usize || opened.contains(&entity) {
            named.shown.push((entity, why));
        } else {
            named.folded.push(entity);
        }
    }
    named
}

/// The names of `named` pile by pile: items no farther from one another than a drop scatters
/// what it holds lie in one pile, its column over their middle, and those of one kind that may be
/// taken alike share a line.
pub fn columns(world: &mut World, named: &NamedItems) -> Vec<PlateColumn> {
    let content = world.resource::<Content>().clone();
    // A drop lays its items in a ring the game's scatter from where it fell, so two of one drop
    // lie up to twice that apart.
    let together = Meters(content.built::<GameSettings>().drops.scatter.0 * 2.0);
    let account = item::DropTaker::me(world);
    let eye = session::me(world).and_then(|me| position(world, me));
    let mut lying: Vec<LyingItem> = named
        .shown
        .iter()
        .map(|&(entity, why)| (entity, Some(why)))
        .chain(named.folded.iter().map(|&entity| (entity, None)))
        .filter_map(|(entity, why)| {
            Some(LyingItem {
                entity,
                why,
                dropped: world.get::<DroppedItem>(entity).cloned()?,
                at: position(world, entity)?,
            })
        })
        .collect();
    lying.sort_by_key(|item| item.entity);
    let mut piles: Vec<Vec<&LyingItem>> = Vec::new();
    for item in &lying {
        let (near, apart): (Vec<_>, Vec<_>) =
            std::mem::take(&mut piles).into_iter().partition(|pile| {
                pile.iter()
                    .any(|other| other.at.distance(item.at) <= together)
            });
        piles = apart;
        piles.push(near.into_iter().flatten().chain([item]).collect());
    }
    piles.sort_by_key(|pile| pile.iter().map(|item| item.entity).min());
    let mut columns: Vec<PlateColumn> = Vec::new();
    for mut pile in piles {
        pile.sort_by_key(|item| item.entity);
        let middle = pile.iter().fold(Vec2::ZERO, |sum, item| {
            sum + Vec2::new(item.at.x, item.at.y)
        }) / pile.len() as f32;
        let mut column = PlateColumn {
            over: Pos::new(middle.x, middle.y),
            lines: Vec::new(),
            folded: Vec::new(),
        };
        for item in pile {
            let Some(why) = item.why else {
                column.folded.push(item.entity);
                continue;
            };
            let reservation = world.get::<Reservation>(item.entity).cloned();
            let mine = item::takeable(reservation.as_ref(), &account);
            let whose = whose(world, reservation.as_ref(), &account);
            let line = column.lines.iter_mut().find(|line| {
                line.item == item.dropped.item && line.mine == mine && line.whose == whose
            });
            match line {
                Some(line) => {
                    line.items.push(item.entity);
                    line.count += item.dropped.count;
                    line.why = line.why.max(why);
                }
                None => column.lines.push(PlateLine {
                    item: item.dropped.item,
                    items: vec![item.entity],
                    count: item.dropped.count,
                    why,
                    mine,
                    whose,
                }),
            }
        }
        columns.push(column);
    }
    let away = |world: &World, entity: Entity| {
        eye.zip(position(world, entity))
            .map_or(0.0_f32, |(eye, at)| eye.distance(at).0)
    };
    for column in &mut columns {
        for line in &mut column.lines {
            line.items
                .sort_by(|a, b| away(world, *a).total_cmp(&away(world, *b)));
        }
        column.lines.sort_by(|a, b| {
            let (one, other) = (a.item.get(&content), b.item.get(&content));
            let rank = |def: &item::ItemDef| def.rarity.get(&content).rank;
            rank(other)
                .cmp(&rank(one))
                .then_with(|| one.name.cmp(&other.name))
        });
    }
    columns.retain(|column| !column.lines.is_empty() || !column.folded.is_empty());
    columns
}

/// Whose a drop reserved as `reservation` says is, to `taker`: nobody's to name when it is
/// their own or anyone's, else its holder's, or its holder's party's when that is theirs too.
fn whose(
    world: &mut World,
    reservation: Option<&Reservation>,
    taker: &item::DropTaker,
) -> Option<String> {
    let holder = reservation?.by.filter(|by| Some(*by) != taker.client)?;
    let name = world
        .query::<(&PlayedBy, &Name)>()
        .iter(world)
        .find(|(by, _)| by.client == holder)
        .map(|(_, name)| name.name.clone())?;
    Some(if taker.party.contains(&holder) {
        format!("{name}'s party")
    } else {
        format!("{name}'s")
    })
}

/// The item a click where the pointer rests takes by its plate: the nearest of the plate's
/// items, when they are the player's to take and the plate shows for one of `whys`.
pub fn taken_by_plate(world: &World, whys: &[Named]) -> Option<Entity> {
    let pointed = *world.resource::<PointedItem>();
    let item = pointed
        .item
        .filter(|_| pointed.plate.is_some_and(|why| whys.contains(&why)))?;
    item::takeable(world.get::<Reservation>(item), &item::DropTaker::me(world)).then_some(item)
}

/// Where the plate that names `item` is drawn, when one is.
pub fn plate_of(world: &World, item: Entity) -> Option<Rect> {
    world
        .resource::<DrawnPlates>()
        .plates
        .iter()
        .find(|plate| plate.items.contains(&item))
        .map(|plate| plate.rect)
}

/// Outlines each item in its rarity's colour for as long as it is named: under the pointer, or
/// with every item the player asked the names of, shown or folded. A player who has their items
/// outlined always sees them so as they lie.
fn outline(world: &mut World, named: &NamedItems) {
    let content = world.resource::<Content>().clone();
    let drops = content.built::<GameSettings>().drops;
    let always = world.resource::<UserSettings>().outlines_items_always();
    let account = item::DropTaker::me(world);
    let asked = world.resource::<NamingItems>().0;
    let pointed = world.resource::<PointedItem>().item;
    let changed: Vec<(Entity, Option<Outline>)> = world
        .query_filtered::<(Entity, &DroppedItem, Option<&Reservation>, Option<&Outline>), Without<DropAnim>>()
        .iter(world)
        .filter_map(|(entity, dropped, reservation, drawn)| {
            let why = named
                .shown
                .iter()
                .find(|(named, _)| *named == entity)
                .map(|(_, why)| *why);
            let folded = asked && named.folded.contains(&entity);
            let width = match why {
                Some(_) if pointed == Some(entity) => Some(drops.outline_hover),
                Some(Named::ByKey) => Some(drops.outline),
                Some(Named::ByPointer | Named::Unprompted) | None => (folded
                    || always && item::takeable(reservation, &account))
                .then_some(drops.outline),
            };
            let wanted = width.map(|width| Outline {
                color: dropped.item.get(&content).rarity.get(&content).gradient.from.color(),
                width,
            });
            (wanted.as_ref() != drawn).then_some((entity, wanted))
        })
        .collect();
    for (entity, wanted) in changed {
        match wanted {
            Some(outline) => world.entity_mut(entity).insert(outline),
            None => world.entity_mut(entity).remove::<Outline>(),
        };
    }
}

/// An item lying on the ground, named or folded, as its pile is made up.
struct LyingItem {
    entity: Entity,
    why: Option<Named>,
    dropped: DroppedItem,
    at: Pos<Meters>,
}

/// The pile whose folded names the pointer opened, for as long as it stays on the pile's column.
#[derive(Resource, Default)]
struct OpenedPile(Vec<Entity>);

/// What was drawn last: each plate with the items it names, and each column with its order, so
/// a column the pointer is on keeps it.
#[derive(Resource, Default)]
struct DrawnPlates {
    plates: Vec<DrawnPlate>,
    columns: Vec<DrawnColumn>,
}

struct DrawnPlate {
    rect: Rect,
    items: Vec<Entity>,
    why: Named,
}

struct DrawnColumn {
    rect: Rect,
    order: Vec<ItemId>,
    chip: Option<Rect>,
    folded: Vec<Entity>,
}

const NAME_SIZE: f32 = 13.0;
const PLAIN_NAME_SIZE: f32 = 11.0;
const ICON: f32 = 16.0;
const GAP: f32 = 4.0;
const PADDING: Vec2 = Vec2::new(7.0, 3.0);
const LINE_GAP: f32 = 2.0;
const EDGE: f32 = 1.5;
const NAME_WIDEST: f32 = 190.0;
const OVER_PILE: Meters = Meters(0.7);
/// How small and how pale a plate at the edge of the key's reach is, against one at the
/// player's feet.
const FAR_SCALE: f32 = 0.82;
const FAR_OPACITY: f32 = 0.7;
/// How long a name that has run out takes to fade away.
const NAME_FADES: Seconds = Seconds(0.6);
const MUTED: Color32 = Color32::from_rgb(150, 150, 156);
const TIE: Color32 = Color32::from_rgba_premultiplied(70, 70, 70, 70);

/// How strongly an item's rarity names it by itself, of 1, if it does: wholly for as long as the
/// name lasts, counted from when the item landed or the pointer last rested on its pile, and
/// ever fainter while it then fades away.
fn names_itself(
    rarity: &ItemRarityDef,
    landed: Option<&Landed>,
    rested: Option<&Rested>,
    near: bool,
    now: Seconds,
) -> Option<f32> {
    match rarity.dropped.named {
        DropNaming::OnKey => None,
        DropNaming::Always => near.then_some(1.0),
        DropNaming::For { seconds } => {
            let lasts = landed.map(|landed| landed.0 + seconds);
            let until = match (lasts, rested.map(|rested| rested.0)) {
                (Some(lasts), Some(rested)) if rested > lasts => rested,
                (lasts, rested) => lasts.or(rested)?,
            };
            let faded = (now - until).ratio(NAME_FADES).clamp(0.0, 1.0);
            (faded < 1.0).then_some(1.0 - faded)
        }
    }
}

/// How strongly the plate of `item` shows, of 1: a name its rarity gave it fades as it runs out.
fn strength(world: &World, content: &Content, item: Entity, near: bool, now: Seconds) -> f32 {
    world
        .get::<DroppedItem>(item)
        .and_then(|dropped| {
            names_itself(
                dropped.item.get(content).rarity.get(content),
                world.get::<Landed>(item),
                world.get::<Rested>(item),
                near,
                now,
            )
        })
        .unwrap_or(0.0)
}

fn read_naming_key(world: &mut World) {
    let naming = world.resource::<UserSettings>().item_naming();
    if *world.resource::<ItemNaming>() != naming {
        world.insert_resource(naming);
    }
    let on = if naming.switched {
        if map::just_pressed(world, InputAction::NameItems) {
            world.resource_mut::<UserSettings>().switch_item_names();
        }
        world.resource::<UserSettings>().item_names_on()
    } else {
        map::pressed(world, InputAction::NameItems)
    };
    if world.resource::<NamingItems>().0 != on {
        world.insert_resource(NamingItems(on));
    }
}

fn forget_plates(world: &mut World) {
    world.insert_resource(DrawnPlates::default());
    world.insert_resource(PointedItem::default());
    world.insert_resource(OpenedPile::default());
}

fn draw_plates(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let pointer = gui
        .pointer_latest_pos()
        .filter(|_| render::pointed(world).is_some());
    point(world, pointer);
    let named = named(world);
    let columns = columns(world, &named);
    let content = world.resource::<Content>().clone();
    let reach = content.built::<GameSettings>().drops.names_reach;
    let eye = session::me(world).and_then(|me| position(world, me));
    let pointed = *world.resource::<PointedItem>();
    let full = bag_refuses(world, &content, pointed.item);
    let now = time::server_now(world);
    let uptime = time::uptime(world);
    let fades_for = content.built::<GameSettings>().drops.fades_for;
    let screen = gui.content_rect();
    let mut drawn = DrawnPlates::default();
    let mut laid: Vec<LaidColumn> = Vec::new();
    for column in &columns {
        let Some(foot) = window_point(world, column.over, Meters(0.0)) else {
            continue;
        };
        let Some(over) = window_point(world, column.over, OVER_PILE) else {
            continue;
        };
        let far = eye.map_or(0.0, |eye| {
            (eye.distance(column.over).0 / reach.0.max(f32::EPSILON)).clamp(0.0, 1.0)
        });
        let scale = 1.0 + (FAR_SCALE - 1.0) * far;
        let opacity = 1.0 + (FAR_OPACITY - 1.0) * far;
        let near = far < 1.0;
        let mut lines: Vec<PlateLook> = column
            .lines
            .iter()
            .map(|line| {
                let def = line.item.get(&content);
                let rarity = def.rarity.get(&content);
                let hovered = pointed.item.is_some_and(|item| line.items.contains(&item));
                let leaving = line.items.iter().any(|item| {
                    world.get::<DroppedItem>(*item).is_some_and(|dropped| {
                        now.is_some_and(|now| dropped.until.since(now) < fades_for)
                    })
                });
                PlateLook {
                    item: line.item,
                    items: line.items.clone(),
                    why: line.why,
                    name: def.name.clone(),
                    count: line.count,
                    icon: rarity
                        .icon
                        .as_ref()
                        .and_then(|icon| gui::image(world, icon))
                        .map(|icon| icon.texture),
                    colors: rarity.gradient.colors(),
                    mine: line.mine,
                    hovered,
                    leaving,
                    strength: match line.why {
                        Named::Unprompted => line
                            .items
                            .iter()
                            .map(|item| strength(world, &content, *item, near, uptime))
                            .fold(0.0, f32::max),
                        Named::ByKey | Named::ByPointer => 1.0,
                    },
                    note: (hovered && full)
                        .then(|| content.built::<GameSettings>().words.needs_a_slot.clone())
                        .or_else(|| line.whose.clone()),
                    scale,
                }
            })
            .collect();
        let was = world
            .resource::<DrawnPlates>()
            .columns
            .iter()
            .find(|was| {
                pointer.is_some_and(|pointer| was.rect.contains(pointer))
                    && lines.iter().any(|line| was.order.contains(&line.item))
            })
            .map(|was| was.order.clone());
        if let Some(order) = was {
            lines.sort_by_key(|line| order.iter().position(|item| *item == line.item));
        }
        let sizes: Vec<Vec2> = lines.iter().map(|line| line.size(&gui)).collect();
        let kinds: BTreeSet<ItemId> = column
            .folded
            .iter()
            .filter_map(|item| Some(world.get::<DroppedItem>(*item)?.item))
            .collect();
        let chip = (!kinds.is_empty()).then(|| chip_size(&gui, kinds.len(), scale));
        let width = sizes
            .iter()
            .chain(&chip)
            .map(|size| size.x)
            .fold(0.0, f32::max);
        let height = sizes.iter().chain(&chip).map(|size| size.y).sum::<f32>()
            + LINE_GAP * (sizes.len() + usize::from(chip.is_some())).saturating_sub(1) as f32;
        let wanted = Rect::from_min_size(
            Pos2::new(over.x - width / 2.0, over.y - height),
            Vec2::new(width, height),
        );
        let rect = wanted.translate(kept_inside(wanted, screen));
        let mut top = rect.top();
        let mut placed = Vec::new();
        for (line, size) in lines.into_iter().zip(sizes) {
            let at = Rect::from_min_size(Pos2::new(rect.center().x - size.x / 2.0, top), size);
            top += size.y + LINE_GAP;
            placed.push((at, line));
        }
        let chip = chip.map(|size| {
            let at = Rect::from_min_size(Pos2::new(rect.center().x - size.x / 2.0, top), size);
            (at, kinds.len())
        });
        drawn.columns.push(DrawnColumn {
            rect,
            order: placed.iter().map(|(_, line)| line.item).collect(),
            chip: chip.map(|(at, _)| at),
            folded: column.folded.clone(),
        });
        drawn
            .plates
            .extend(placed.iter().map(|(at, line)| DrawnPlate {
                rect: *at,
                items: line.items.clone(),
                why: line.why,
            }));
        laid.push(LaidColumn {
            rect,
            foot: Pos2::new(foot.x, foot.y),
            opacity,
            scale,
            lines: placed,
            chip,
        });
    }
    gui::world_area(world, "item plates")
        .fixed_pos(Pos2::ZERO)
        .show(&gui, |ui| {
            for column in &laid {
                ui.set_opacity(column.opacity);
                ui.painter().line_segment(
                    [
                        Pos2::new(column.rect.center().x, column.rect.bottom()),
                        column.foot,
                    ],
                    Stroke::new(1.0, TIE),
                );
                if let Some((at, kinds)) = column.chip {
                    paint_chip(ui.painter(), at, kinds, column.scale);
                }
                for (at, line) in &column.lines {
                    ui.set_opacity(column.opacity * line.strength);
                    line.paint(ui.painter(), *at);
                }
            }
        });
    open_pile(world, &drawn, pointer);
    rest_on_pile(world, &drawn, pointer, uptime);
    world.insert_resource(drawn);
    outline(world, &named);
}

/// Finds the item the pointer rests on: the nearest named by the plate under it, or else the
/// one lying where it points.
fn point(world: &mut World, pointer: Option<Pos2>) {
    let on_plate = pointer.and_then(|pointer| {
        world
            .resource::<DrawnPlates>()
            .plates
            .iter()
            .find(|plate| plate.rect.contains(pointer))
            .and_then(|plate| Some((*plate.items.first()?, plate.why)))
    });
    let pointed = match on_plate {
        Some((item, why)) => PointedItem {
            item: Some(item),
            plate: Some(why),
        },
        None => PointedItem {
            item: render::pointed(world)
                .and_then(|pointed| pointed.ground)
                .and_then(|ground| item::lying_at(world, ground)),
            plate: None,
        },
    };
    if *world.resource::<PointedItem>() != pointed {
        world.insert_resource(pointed);
    }
}

/// Opens the pile whose chip the pointer rests on, and keeps it open while the pointer stays on
/// its column.
fn open_pile(world: &mut World, drawn: &DrawnPlates, pointer: Option<Pos2>) {
    let opened = pointer.and_then(|pointer| {
        let on_chip = drawn
            .columns
            .iter()
            .find(|column| column.chip.is_some_and(|chip| chip.contains(pointer)));
        let kept = || {
            let opened = &world.resource::<OpenedPile>().0;
            drawn
                .columns
                .iter()
                .any(|column| column.rect.contains(pointer))
                .then(|| opened.clone())
        };
        on_chip.map(|column| column.folded.clone()).or_else(kept)
    });
    world.insert_resource(OpenedPile(opened.unwrap_or_default()));
}

/// Notes the pointer resting on the names of a pile, on every item named there.
fn rest_on_pile(world: &mut World, drawn: &DrawnPlates, pointer: Option<Pos2>, now: Seconds) {
    let Some(column) = pointer.and_then(|pointer| {
        drawn
            .columns
            .iter()
            .find(|column| column.rect.contains(pointer))
    }) else {
        return;
    };
    let named: Vec<Entity> = drawn
        .plates
        .iter()
        .filter(|plate| column.rect.contains(plate.rect.center()))
        .flat_map(|plate| plate.items.iter().copied())
        .collect();
    for item in named {
        if let Ok(mut item) = world.get_entity_mut(item) {
            item.insert(Rested(now));
        }
    }
}

/// Whether the player's bag has no room for `item`, as taking it would find out.
fn bag_refuses(world: &World, content: &Content, item: Option<Entity>) -> bool {
    let Some(dropped) = item.and_then(|item| world.get::<DroppedItem>(item)) else {
        return false;
    };
    session::my_character(world)
        .and_then(|me| me.get::<Inventory>().cloned())
        .is_some_and(|mut bag| {
            bag.exchange(content, &[], &[ItemStack::new(dropped.item, dropped.count)])
                .is_err()
        })
}

/// How far to move `rect` so that it lies inside `screen`.
fn kept_inside(rect: Rect, screen: Rect) -> Vec2 {
    let along = |min: f32, max: f32, low: f32, high: f32| {
        if min < low {
            low - min
        } else if max > high {
            high - max
        } else {
            0.0
        }
    };
    Vec2::new(
        along(rect.left(), rect.right(), screen.left(), screen.right()),
        along(rect.top(), rect.bottom(), screen.top(), screen.bottom()),
    )
}

/// A pile's column as it is drawn: where it lies, the point on the ground it is tied to, and its
/// plates with the chip under them that counts the kinds left unnamed.
struct LaidColumn {
    rect: Rect,
    foot: Pos2,
    opacity: f32,
    scale: f32,
    lines: Vec<(Rect, PlateLook)>,
    chip: Option<(Rect, usize)>,
}

/// One plate as it is drawn.
struct PlateLook {
    item: ItemId,
    items: Vec<Entity>,
    why: Named,
    name: String,
    count: u32,
    icon: Option<egui::TextureId>,
    colors: [Color32; 2],
    mine: bool,
    hovered: bool,
    leaving: bool,
    /// How strongly it shows, of 1.
    strength: f32,
    note: Option<String>,
    scale: f32,
}

impl PlateLook {
    fn font(&self) -> FontId {
        let size = if self.icon.is_some() {
            NAME_SIZE
        } else {
            PLAIN_NAME_SIZE
        };
        Face::BookBold.id(size * self.scale)
    }

    fn text(&self) -> String {
        if self.count > 1 {
            format!("{} \u{d7}{}", self.name, self.count)
        } else {
            self.name.clone()
        }
    }

    fn mark(&self) -> Option<Mark> {
        if !self.mine {
            Some(Mark::Lock)
        } else if self.leaving {
            Some(Mark::Hourglass)
        } else {
            None
        }
    }

    fn side(&self) -> f32 {
        ICON * self.scale
    }

    fn size(&self, gui: &egui::Context) -> Vec2 {
        let text = paint::text_size(gui, &self.text(), self.font());
        let note = self
            .note
            .as_ref()
            .map(|note| paint::text_size(gui, note, self.note_font()));
        let lead = if self.icon.is_some() {
            self.side() + GAP
        } else {
            0.0
        };
        let trail = if self.mark().is_some() {
            self.side() + GAP
        } else {
            0.0
        };
        let line = Vec2::new(
            lead + text.x.min(NAME_WIDEST) + trail,
            text.y.max(if self.icon.is_some() {
                self.side()
            } else {
                0.0
            }),
        );
        Vec2::new(
            line.x.max(note.map_or(0.0, |note| note.x)),
            line.y + note.map_or(0.0, |note| note.y),
        ) + PADDING * 2.0
    }

    fn note_font(&self) -> FontId {
        Face::Book.id(PLAIN_NAME_SIZE * self.scale)
    }

    fn paint(&self, painter: &egui::Painter, rect: Rect) {
        dusk::plate(painter, rect, 0.85);
        let ink = if self.mine {
            paint::gradient_edge(painter, rect, EDGE, self.colors);
            ink::legible(self.colors[0], dusk::FILL)
        } else {
            paint::dashed_rect(painter, rect, Stroke::new(1.0, MUTED), 3.0, 3.0);
            MUTED
        };
        if self.hovered {
            painter.rect_stroke(
                rect.expand(1.5),
                dusk::RADIUS,
                Stroke::new(1.0, Color32::WHITE),
                StrokeKind::Outside,
            );
        }
        let inner = rect.shrink2(PADDING);
        let note_height = self.note.as_ref().map_or(0.0, |note| {
            paint::text_size(painter.ctx(), note, self.note_font()).y
        });
        let line = Rect::from_min_max(inner.min, Pos2::new(inner.max.x, inner.max.y - note_height));
        let mut left = line.left();
        if let Some(icon) = self.icon {
            let tint = if self.mine { Color32::WHITE } else { MUTED };
            painter.image(
                icon,
                Rect::from_center_size(
                    Pos2::new(left + self.side() / 2.0, line.center().y),
                    Vec2::splat(self.side()),
                ),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                tint,
            );
            left += self.side() + GAP;
        }
        let name = paint::text_within(
            painter,
            Pos2::new(left, line.center().y),
            Align2::LEFT_CENTER,
            &self.text(),
            self.font(),
            ink,
            NAME_WIDEST,
        );
        if let Some(mark) = self.mark() {
            mark.paint(
                painter,
                Rect::from_center_size(
                    Pos2::new(name.right() + GAP + self.side() / 2.0, line.center().y),
                    Vec2::splat(self.side()),
                ),
                ink,
            );
        }
        if let Some(note) = &self.note {
            painter.text(
                Pos2::new(inner.left(), inner.bottom()),
                Align2::LEFT_BOTTOM,
                note,
                self.note_font(),
                dusk::DANGER,
            );
        }
    }
}

fn chip_text(kinds: usize) -> String {
    format!("+{kinds} more")
}

fn chip_font(scale: f32) -> FontId {
    Face::Book.id(PLAIN_NAME_SIZE * scale)
}

fn chip_size(gui: &egui::Context, kinds: usize, scale: f32) -> Vec2 {
    paint::text_size(gui, &chip_text(kinds), chip_font(scale)) + PADDING * 2.0
}

fn paint_chip(painter: &egui::Painter, rect: Rect, kinds: usize, scale: f32) {
    dusk::plate(painter, rect, 0.7);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        chip_text(kinds),
        chip_font(scale),
        dusk::TEXT,
    );
}
