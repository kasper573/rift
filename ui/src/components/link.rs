use bevy::prelude::*;
use bevy_scene::{EntityScene, Scene, bsn, on};

use crate::components::tooltip::{TooltipText, tooltip, tooltip_content, tooltip_text};
use crate::drag::OnTap;
use crate::{Align, Side, component};

#[derive(Clone)]
pub struct LinkOptions {
    pub tooltip: TooltipText,
    pub on_tap: OnTap,
}

pub fn link(options: LinkOptions, content: impl Scene) -> impl Scene {
    let LinkOptions {
        tooltip: text,
        on_tap,
    } = options;
    bsn! {
        {tooltip(false)}
        Node
        component(on_tap)
        Pickable { should_block_lower: true, is_hoverable: true }
        on(follow)
        Children [
            {EntityScene(content)},
            (
                {tooltip_content(Side::Top, Align::Center, 4.0)}
                Children [ {EntityScene(tooltip_text(text))} ]
            ),
        ]
    }
}

fn follow(mut click: On<Pointer<Click>>, taps: Query<&OnTap>, mut commands: Commands) {
    if click.button != PointerButton::Primary {
        return;
    }
    let Ok(tap) = taps.get(click.entity) else {
        return;
    };
    click.propagate(false);
    let tap = tap.0.clone();
    commands.queue(move |world: &mut World| tap(world));
}
