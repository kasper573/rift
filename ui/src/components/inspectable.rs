use bevy::prelude::*;
use bevy_scene::{EntityScene, Scene, bsn, on};

use crate::components::tooltip::{TooltipText, tooltip, tooltip_content, tooltip_text};
use crate::drag::OnTap;
use crate::{Align, Side, component};

#[derive(Clone)]
pub struct InspectableOptions {
    pub tooltip: TooltipText,
    pub on_inspect: OnTap,
}

pub fn inspectable(options: InspectableOptions, content: impl Scene) -> impl Scene {
    let InspectableOptions {
        tooltip: text,
        on_inspect,
    } = options;
    bsn! {
        {tooltip(false)}
        Node
        component(on_inspect)
        Pickable { should_block_lower: true, is_hoverable: true }
        on(inspect)
        Children [
            {EntityScene(content)},
            (
                {tooltip_content(Side::Top, Align::Center, 4.0)}
                Children [ {EntityScene(tooltip_text(text))} ]
            ),
        ]
    }
}

fn inspect(mut click: On<Pointer<Click>>, taps: Query<&OnTap>, mut commands: Commands) {
    if click.button != PointerButton::Secondary {
        return;
    }
    let Ok(tap) = taps.get(click.entity) else {
        return;
    };
    click.propagate(false);
    let tap = tap.0.clone();
    commands.queue(move |world: &mut World| tap(world));
}
