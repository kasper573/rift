//! Portraits: a still of an actor's head as its appearance alone makes it, with nothing worn,
//! for the interface to show beside what the actor says.

use std::collections::{BTreeMap, VecDeque};

use bevy::prelude::*;
use ui::egui;

use super::Appearance;
use super::appearance::look;
use super::model::ActorModel;
use crate::core::content::Content;
use crate::core::content::gltf::MeshBounds;
use crate::core::content::media::GltfInfos;
use crate::core::render::world::body::{self, Figure};
use crate::core::stage::still::{self, StillFrame};
use crate::systems::screen::ClientScreen;

pub struct PortraitPlugin;

impl Plugin for PortraitPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActorPortraits>()
            .add_systems(Update, sit.run_if(in_state(ClientScreen::Area)));
    }
}

/// The portraits taken so far, by the appearance each is of, and those still to take, one at a
/// time. An appearance whose skeleton shows nothing in a portrait is kept without one.
#[derive(Resource, Default)]
pub struct ActorPortraits {
    taken: Vec<(Appearance, Option<egui::TextureId>)>,
    asked: VecDeque<Appearance>,
    sitting: Option<PortraitSitting>,
}

impl ActorPortraits {
    /// The portrait of an actor looking as `appearance`, once it is taken.
    pub fn of(&self, appearance: &Appearance) -> Option<egui::TextureId> {
        self.taken
            .iter()
            .find(|(of, _)| of == appearance)
            .and_then(|(_, picture)| *picture)
    }

    /// Whether an actor looking as `appearance` has a portrait, taken or still to take.
    pub fn has(&self, appearance: &Appearance) -> bool {
        !self
            .taken
            .iter()
            .any(|(of, picture)| of == appearance && picture.is_none())
    }

    /// Has the portrait of an actor looking as `appearance` taken, unless it is taken or on its
    /// way.
    pub fn ask(&mut self, appearance: &Appearance) {
        let known = self.taken.iter().any(|(of, _)| of == appearance)
            || self.asked.contains(appearance)
            || self
                .sitting
                .as_ref()
                .is_some_and(|sitting| sitting.of == *appearance);
        if !known {
            self.asked.push_back(appearance.clone());
        }
    }
}

/// A portrait being taken: the still, and the head standing under it.
struct PortraitSitting {
    of: Appearance,
    still: Entity,
    head: Entity,
    frames: u32,
}

/// Takes the portrait being sat for once its head stands whole, and sits the next one asked for.
fn sit(world: &mut World) {
    let portraits = world.resource::<ActorPortraits>();
    if portraits.sitting.is_none() && portraits.asked.is_empty() {
        return;
    }
    world.resource_scope(|world, mut portraits: Mut<ActorPortraits>| {
        if let Some(mut sitting) = portraits.sitting.take() {
            sitting.frames += 1;
            let whole = stands_whole(world, sitting.head);
            if let Some(picture) = still::take(world, sitting.still, whole) {
                portraits.taken.push((sitting.of, Some(picture)));
            } else if sitting.frames > GIVE_UP_FRAMES {
                still::strike(world, sitting.still);
                portraits.taken.push((sitting.of, None));
            } else {
                portraits.sitting = Some(sitting);
            }
            return;
        }
        let Some(of) = portraits.asked.pop_front() else {
            return;
        };
        let sat = head(world.resource::<Content>(), &of)
            .and_then(|(figure, frame)| Some((figure, still::stand(world, frame)?)));
        let Some((figure, still)) = sat else {
            portraits.taken.push((of, None));
            return;
        };
        let head = world
            .spawn((Transform::default(), Visibility::default(), ChildOf(still)))
            .id();
        body::dress(world, head, &figure);
        portraits.sitting = Some(PortraitSitting {
            of,
            still,
            head,
            frames: 0,
        });
    });
}

/// What a portrait of `appearance` shows, and the square it fits from the front: of the parts
/// the appearance alone lays on its model, those that cover a region its skeleton's portrait
/// shows.
fn head(content: &Content, appearance: &Appearance) -> Option<(Figure, StillFrame)> {
    let model = ActorModel::loaded(content, appearance.model)?;
    let shown = &model.skeleton().def.portrait;
    let mut bare = look(content, appearance, &BTreeMap::new());
    bare.parts.retain(|laid| {
        let covers = &laid.part.get(content).covers;
        covers.iter().any(|region| shown.contains(region))
    });
    let figure = model.dressed(content, &bare);
    let models = &content.built::<GltfInfos>().0;
    let bounds = figure
        .skins
        .iter()
        .filter_map(|skin| models.get(&skin.mesh))
        .map(|info| info.bounds)
        .reduce(|one, other| MeshBounds {
            min: one.min.min(other.min),
            max: one.max.max(other.max),
        })?;
    let size = bounds.max - bounds.min;
    let frame = StillFrame {
        center: (bounds.min + bounds.max) / 2.0,
        side: size.x.max(size.y) * ROOM,
    };
    Some((figure, frame))
}

/// Whether every skin of the body standing under `head` is bound to its rig and shown.
fn stands_whole(world: &World, head: Entity) -> bool {
    let Some(children) = world.get::<Children>(head) else {
        return false;
    };
    let mut skins = children
        .iter()
        .filter(|child| world.get::<WorldAssetRoot>(*child).is_some())
        .peekable();
    skins.peek().is_some()
        && skins.all(|skin| world.get::<Visibility>(skin) != Some(&Visibility::Hidden))
}

/// How much wider than the head its portrait is, so that no part of it touches the edge.
const ROOM: f32 = 1.1;
/// How many frames a head whose skins never load is waited on.
const GIVE_UP_FRAMES: u32 = 600;
