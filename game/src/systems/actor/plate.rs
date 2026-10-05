use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use bevy::prelude::*;
use bevy::scene::EntityScene;
use bevy::ui::Val2;
use ui::tokens::{palette, typography};

use crate::core::math::Pos;
use crate::core::render::WorldToWindow;
use crate::systems::actor::{Action, Actor, Hitbox, Name};
use crate::systems::attention::{self, Mark};
use crate::systems::combat::Attitude;
use crate::systems::hud::{reconcile_children, tooltip_label};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::npc::Npc;
use crate::systems::player::Owner;
use crate::systems::player::session::Viewpoint;
use crate::systems::prop::Prop;
use crate::systems::scene::Scene as GameScene;
use bevy::ecs::query::QueryData;

const HEAD_ROOM: f32 = 6.0;
const ICON: f32 = 22.0;

pub struct PlatePlugin;

impl Plugin for PlatePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Plates>()
            .add_systems(
                Update,
                (spawn_plates, place_plates, sync_plate_icons)
                    .chain()
                    .run_if(in_state(GameScene::Area)),
            )
            .add_systems(OnExit(GameScene::Area), clear_plates);
    }
}

#[derive(Component, Default, Clone)]
pub struct WorldOverlay;

#[derive(Resource, Default)]
struct Plates(HashMap<Entity, Entity>);

#[derive(Component, Default, Clone)]
struct PlateIcons;

type Labelled = (
    Entity,
    Option<&'static Name>,
    Option<&'static Npc>,
    Option<&'static Owner>,
    Option<&'static Attitude>,
    Has<Prop>,
);

type Plated = Or<(With<Actor>, With<Prop>)>;

struct Label {
    name: Option<String>,
    role: Option<&'static str>,
    color: Color,
}

fn label(
    (_, name, npc, owner, attitude, prop): <Labelled as QueryData>::Item<'_, '_>,
) -> Option<Label> {
    if prop {
        return Some(Label {
            name: None,
            role: None,
            color: Color::WHITE,
        });
    }
    match (npc, owner) {
        (Some(npc), _) => {
            let def = npc.def.get();
            def.role.map(|role| Label {
                name: Some(def.display_name.to_owned()),
                role: Some(role),
                color: match attitude {
                    Some(Attitude::Hostile) => palette::CRIMSON_80,
                    _ => Color::WHITE,
                },
            })
        }
        (None, Some(_)) => Some(Label {
            name: Some(name?.name.clone()),
            role: None,
            color: palette::AZURE_80,
        }),
        (None, None) => None,
    }
}

fn spawn_plates(
    mut plates: ResMut<Plates>,
    viewpoint: Res<Viewpoint>,
    actors: Query<Labelled, Plated>,
    mut commands: Commands,
) {
    plates.0.retain(|actor, plate| {
        let keep = actors.contains(*actor) && viewpoint.0 != Some(*actor);
        if !keep {
            commands.entity(*plate).despawn();
        }
        keep
    });
    for labelled in &actors {
        let actor = labelled.0;
        if plates.0.contains_key(&actor) || viewpoint.0 == Some(actor) {
            continue;
        }
        let Some(label) = label(labelled) else {
            continue;
        };
        let plate = commands.spawn_scene(plate(label)).id();
        plates.0.insert(actor, plate);
    }
}

fn place_plates(
    plates: Res<Plates>,
    actors: Query<(Option<&RenderPosition>, &Position, &Hitbox, Option<&Actor>)>,
    projector: WorldToWindow,
    mut nodes: Query<(&mut Node, &mut Visibility), With<WorldOverlay>>,
) {
    for (&actor, &plate) in &plates.0 {
        let Ok((mut node, mut visibility)) = nodes.get_mut(plate) else {
            continue;
        };
        let head = actors
            .get(actor)
            .ok()
            .and_then(|(rendered, at, hitbox, body)| {
                let at = rendered.map_or(at.pos, |rendered| rendered.0);
                let top = Pos::new(at.x, at.y + 0.5 - hitbox.size.height);
                body.is_none_or(|body| body.action != Action::Dead)
                    .then(|| projector.project(top))
                    .flatten()
            });
        let shown = match head {
            Some(head) => {
                let left = Val::Px(head.x.round());
                let top = Val::Px((head.y - HEAD_ROOM).round());
                if node.left != left || node.top != top {
                    node.left = left;
                    node.top = top;
                }
                Visibility::Inherited
            }
            None => Visibility::Hidden,
        };
        if *visibility != shown {
            *visibility = shown;
        }
    }
}

fn sync_plate_icons(world: &mut World) {
    let viewer = world.resource::<Viewpoint>().0;
    let plates: Vec<(Entity, Entity)> = world
        .resource::<Plates>()
        .0
        .iter()
        .map(|(&actor, &plate)| (actor, plate))
        .collect();
    for (actor, plate) in plates {
        let Some(row) = world.get::<Children>(plate).and_then(|kids| {
            kids.iter()
                .find(|kid| world.get::<PlateIcons>(*kid).is_some())
        }) else {
            continue;
        };
        let marks = attention::plate_marks(world, viewer, actor);
        let keys: Vec<u64> = marks.iter().map(mark_key).collect();
        let icons: Vec<(Handle<Image>, String)> = {
            let assets = world.resource::<AssetServer>();
            marks
                .iter()
                .map(|mark| (assets.load(mark.kind.get().icon.0), mark.label.clone()))
                .collect()
        };
        reconcile_children(world, row, &keys, |index| {
            let (image, label) = icons[index].clone();
            Box::new(plate_icon(image, label))
        });
    }
}

fn mark_key(mark: &Mark) -> u64 {
    let mut hasher = DefaultHasher::new();
    mark.kind.hash(&mut hasher);
    mark.label.hash(&mut hasher);
    hasher.finish()
}

fn plate_icon(image: Handle<Image>, label: String) -> impl Scene {
    bsn! {
        Node { width: Val::Px({ICON}), height: Val::Px({ICON}) }
        ImageNode { image: {image} }
        Pickable { should_block_lower: false, is_hoverable: true }
        {ui::tooltip(false)}
        Children [
            (
                {ui::tooltip_content(ui::Side::Top, ui::Align::Center, 4.0)}
                Children [ {EntityScene(tooltip_label(label))} ]
            )
        ]
    }
}

fn clear_plates(mut plates: ResMut<Plates>, mut commands: Commands) {
    for (_, plate) in plates.0.drain() {
        commands.entity(plate).despawn();
    }
}

fn plate(label: Label) -> impl Scene {
    let shadow = TextShadow {
        offset: Vec2::new(1.0, 1.0),
        color: Color::BLACK,
    };
    let name = label.name.map(|name| {
        bsn! {
            {ui::styled_text(name, label.color, typography::LABEL)}
            ui::component(shadow)
        }
    });
    let role = label.role.map(|role| {
        bsn! {
            {ui::styled_text(format!("<{role}>"), Color::WHITE.with_alpha(0.7), typography::CAPTION)}
            ui::component(shadow)
        }
    });
    bsn! {
        WorldOverlay
        Node {
            position_type: PositionType::Absolute,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
        }
        UiTransform { translation: {Val2::percent(-50.0, -100.0)} }
        GlobalZIndex(-1)
        Pickable::IGNORE
        Children [
            ( PlateIcons Node { column_gap: Val::Px(2.0) } Pickable::IGNORE ),
            {name},
            {role},
        ]
    }
}
