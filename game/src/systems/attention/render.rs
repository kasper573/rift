use bevy::prelude::*;
use bevy::scene::EntityScene;
use bevy::window::PrimaryWindow;
use ui::tokens::typography;

use super::{Attention, Mark};
use crate::core::render;
use crate::systems::actor::Name;
use crate::systems::actor::plate::WorldOverlay;
use crate::systems::input::gestures;
use crate::systems::interact;
use crate::systems::npc::Npc;
use crate::systems::player::session::{self, Viewpoint};
use crate::systems::scene::Scene as GameScene;

const CURSOR_GAP: Vec2 = Vec2::new(20.0, 20.0);
const ICON: f32 = 18.0;

pub struct OfferingsPlugin;

impl Plugin for OfferingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Offered>()
            .add_systems(Update, show_offerings.run_if(in_state(GameScene::Area)))
            .add_systems(
                OnExit(GameScene::Area),
                (crate::systems::scene::despawn_all::<OfferingsTip>, forget),
            );
    }
}

#[derive(Resource, Default)]
struct Offered(Option<(Entity, Vec<Mark>)>);

#[derive(Component, Default, Clone)]
struct OfferingsTip;

fn show_offerings(world: &mut World) {
    let offered = hovered(world).map(|target| {
        let marks = world
            .resource::<Viewpoint>()
            .0
            .and_then(|viewer| world.get::<Attention>(viewer))
            .map(|attention| attention.of(target).to_vec())
            .unwrap_or_default();
        (target, marks)
    });
    if world.resource::<Offered>().0 != offered {
        let tips: Vec<Entity> = world
            .query_filtered::<Entity, With<OfferingsTip>>()
            .iter(world)
            .collect();
        for tip in tips {
            world.entity_mut(tip).despawn();
        }
        if let Some(scene) = offered
            .as_ref()
            .and_then(|(target, marks)| tip(world, *target, marks))
        {
            let _ = world.spawn_scene(scene);
        }
        world.resource_mut::<Offered>().0 = offered;
    }
    let Some(cursor) = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()
        .and_then(Window::cursor_position)
    else {
        return;
    };
    let at = cursor + CURSOR_GAP;
    let mut tips = world.query_filtered::<&mut Node, With<OfferingsTip>>();
    for mut node in tips.iter_mut(world) {
        let (left, top) = (Val::Px(at.x.round()), Val::Px(at.y.round()));
        if node.left != left || node.top != top {
            node.left = left;
            node.top = top;
        }
    }
}

fn forget(mut offered: ResMut<Offered>) {
    offered.0 = None;
}

fn hovered(world: &mut World) -> Option<Entity> {
    if !session::is_alive(world) || session::is_locked(world) || gestures::over_interface(world) {
        return None;
    }
    let point = render::cursor_tile(world)?;
    interact::interactable_at(world, point)
}

fn tip(world: &World, target: Entity, marks: &[Mark]) -> Option<Box<dyn Scene>> {
    let verb = interact::interaction_of(world, target)?.verb;
    let name = world.get::<Name>(target)?.name.clone();
    let title = match world.get::<Npc>(target).and_then(|npc| npc.def.get().role) {
        Some(role) => format!("{name} <{role}>"),
        None => name,
    };
    let assets = world.resource::<AssetServer>();
    let rows: Vec<Box<dyn Scene>> = marks
        .iter()
        .map(|mark| (assets.load(mark.kind.get().icon.0), mark.label.clone()))
        .chain(std::iter::once((
            assets.load(verb.cursor()),
            verb.label().to_owned(),
        )))
        .map(|(image, label)| -> Box<dyn Scene> { Box::new(row(image, label)) })
        .collect();
    let title = ui::styled_text(title, Color::WHITE, typography::LABEL);
    Some(Box::new(bsn! {
        OfferingsTip
        WorldOverlay
        Node {
            position_type: PositionType::Absolute,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(3.0),
            padding: {UiRect::axes(Val::Px(8.0), Val::Px(6.0))},
        }
        BackgroundColor({Color::BLACK.with_alpha(0.88)})
        GlobalZIndex(2)
        Pickable::IGNORE
        Children [ {EntityScene(title)}, {rows} ]
    }))
}

fn row(image: Handle<Image>, label: String) -> impl Scene {
    bsn! {
        Node { column_gap: Val::Px(6.0), align_items: AlignItems::Center }
        Pickable::IGNORE
        Children [
            ( Node { width: Val::Px({ICON}), height: Val::Px({ICON}) } ImageNode { image: {image} } ),
            {EntityScene(ui::text_colored(label, Color::WHITE))},
        ]
    }
}
