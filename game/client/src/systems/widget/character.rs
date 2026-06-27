use bevy::prelude::*;
use ui::component;
use ui::{DragHandle, DragRoot, OnSettle, text_colored};
use world::systems::actor::Name;
use world::systems::player::Xp;
use world::systems::player::session;
use world::systems::stat;

#[derive(Component, Default, Clone)]
struct CharacterText;

inventory::submit! {
    super::WidgetDef {
        id: "character",
        fallback: Vec2::new(8.0, 8.0),
        build,
        sync: sync_character,
    }
}

fn build(pos: Vec2, id: &'static str) -> Box<dyn Scene> {
    let node = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(pos.x),
        top: Val::Px(pos.y),
        width: Val::Px(140.0),
        height: Val::Px(64.0),
        border: UiRect::all(Val::Px(1.0)),
        padding: UiRect::all(Val::Px(6.0)),
        ..default()
    };
    Box::new(bsn! {
        template_value(node)
        BackgroundColor({super::PANEL_BG})
        component(BorderColor::all(super::BORDER))
        DragRoot
        DragHandle
        component(OnSettle::new(move |world, geom| super::persist_widget(world, id, geom)))
        Children [ ( {text_colored(String::new(), Color::WHITE)} CharacterText ) ]
    })
}

fn sync_character(world: &mut World) {
    let text = character_text(world);
    let mut query = world.query_filtered::<&mut Text, With<CharacterText>>();
    for mut node in query.iter_mut(world) {
        node.0 = text.clone();
    }
}

fn character_text(world: &World) -> String {
    let Some(me) = session::me(world) else {
        return String::new();
    };
    let entity = me.id();
    let name = me
        .get::<Name>()
        .map_or_else(String::new, |n| n.name.clone());
    let xp = me.get::<Xp>().map_or(0, |x| x.amount);
    let health = stat::current_health(world, entity);
    let max = stat::max_health(world, entity);
    format!("{name}\n{health:.0} / {max:.0}\nxp {xp}")
}
