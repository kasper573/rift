use std::collections::HashMap;

use bevy::prelude::*;
use bevy::ui::Val2;
use ui::tokens::{palette, typography};

use crate::core::math::Pos;
use crate::core::render::WorldToWindow;
use crate::systems::actor::{Action, Actor, Hitbox, Name};
use crate::systems::movement::RenderPosition;
use crate::systems::npc::Npc;
use crate::systems::player::Owner;
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;

const HEAD_ROOM: f32 = 6.0;

pub struct PlatePlugin;

impl Plugin for PlatePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Plates>()
            .add_systems(
                Update,
                (spawn_plates, place_plates)
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

type Labelled = (
    Entity,
    Option<&'static Name>,
    Option<&'static Npc>,
    Option<&'static Owner>,
);

struct Label {
    name: String,
    role: Option<&'static str>,
    color: Color,
}

fn label(name: Option<&Name>, npc: Option<&Npc>, owner: Option<&Owner>) -> Option<Label> {
    match (npc, owner) {
        (Some(npc), _) => {
            let def = npc.def.get();
            def.role.map(|role| Label {
                name: def.display_name.to_owned(),
                role: Some(role),
                color: Color::WHITE,
            })
        }
        (None, Some(_)) => Some(Label {
            name: name.map(|name| name.name.clone())?,
            role: None,
            color: palette::AZURE_80,
        }),
        (None, None) => None,
    }
}

fn spawn_plates(
    mut plates: ResMut<Plates>,
    viewpoint: Res<Viewpoint>,
    actors: Query<Labelled, With<Actor>>,
    mut commands: Commands,
) {
    plates.0.retain(|actor, plate| {
        let keep = actors.contains(*actor) && viewpoint.0 != Some(*actor);
        if !keep {
            commands.entity(*plate).despawn();
        }
        keep
    });
    for (actor, name, npc, owner) in &actors {
        if plates.0.contains_key(&actor) || viewpoint.0 == Some(actor) {
            continue;
        }
        let Some(label) = label(name, npc, owner) else {
            continue;
        };
        let plate = commands.spawn_scene(plate(label)).id();
        plates.0.insert(actor, plate);
    }
}

fn place_plates(
    plates: Res<Plates>,
    actors: Query<(&RenderPosition, &Hitbox, &Actor)>,
    projector: WorldToWindow,
    mut nodes: Query<(&mut Node, &mut Visibility), With<WorldOverlay>>,
) {
    for (&actor, &plate) in &plates.0 {
        let Ok((mut node, mut visibility)) = nodes.get_mut(plate) else {
            continue;
        };
        let head = actors.get(actor).ok().and_then(|(at, hitbox, body)| {
            let top = Pos::new(at.0.x, at.0.y + 0.5 - hitbox.size.height);
            (body.action != Action::Dead)
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
            (
                {ui::styled_text(label.name, label.color, typography::LABEL)}
                ui::component(shadow)
            ),
            {role},
        ]
    }
}
