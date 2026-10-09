use crate::systems::actor::Name;
use crate::systems::player::Xp;
use crate::systems::player::session;
use crate::systems::stat;
use bevy::prelude::*;
use ui::component;
use ui::{DragHandle, DragRoot, OnSettle, text_colored};

#[derive(Component, Default, Clone)]
struct CharacterText;

pub struct CharacterWidget;

impl crate::systems::hud::Widget for CharacterWidget {
    fn audience(&self) -> crate::systems::hud::HudAudience {
        crate::systems::hud::HudAudience::Players
    }
    fn fallback(&self, _: f32) -> Vec2 {
        Vec2::new(8.0, 8.0)
    }
    fn build(&self, pos: Vec2, id: &'static str) -> Box<dyn Scene> {
        build(pos, id)
    }
    fn sync(&self, world: &mut World) {
        sync_character(world)
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
        BackgroundColor({crate::systems::hud::PANEL_BG})
        component(BorderColor::all(crate::systems::hud::BORDER))
        DragRoot
        DragHandle
        component(OnSettle::new(move |world, geom| crate::systems::hud::persist_widget(world, id, geom)))
        Children [ ( {text_colored(String::new(), Color::WHITE)} CharacterText ) ]
    })
}

// Between areas there is no character for a moment; the panel keeps what it last showed.
fn sync_character(world: &mut World) {
    let Some(text) = character_text(world) else {
        return;
    };
    let mut query = world.query_filtered::<&mut Text, With<CharacterText>>();
    for mut node in query.iter_mut(world) {
        node.0 = text.clone();
    }
}

fn character_text(world: &World) -> Option<String> {
    let me = session::my_character(world)?;
    let entity = me.id();
    let name = me
        .get::<Name>()
        .map_or_else(String::new, |n| n.name.clone());
    let xp = me.get::<Xp>().map_or(0, |x| x.amount);
    let health = stat::current_health(world, entity);
    let max = stat::max_health(world, entity);
    Some(format!("{name}\n{health:.0} / {max:.0}\nxp {xp}"))
}
