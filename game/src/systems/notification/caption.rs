use bevy::prelude::*;

use super::rows::{self, NotificationRows, VoiceRows};
use super::{NotificationKind, NotificationSent};
use crate::core::time::Seconds;
use crate::systems::scene::Scene as GameScene;

const ROWS: usize = 3;
const REPLACE_AFTER: Seconds = Seconds(2.0);

pub struct CaptionPlugin;

impl Plugin for CaptionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CaptionBlock>()
            .add_systems(Update, show_captions.run_if(in_state(GameScene::Area)));
    }
}

pub fn join(world: &mut World, notification_sent: NotificationSent) {
    let NotificationKind::Narration { label, .. } = &notification_sent.notification.kind else {
        return;
    };
    let label = label.clone();
    world
        .resource_mut::<CaptionBlock>()
        .0
        .join(label, notification_sent);
}

pub fn shown(world: &World) -> NotificationRows {
    world.resource::<CaptionBlock>().0.shown()
}

#[derive(Resource)]
struct CaptionBlock(VoiceRows);

impl Default for CaptionBlock {
    fn default() -> CaptionBlock {
        CaptionBlock(VoiceRows::new(ROWS, REPLACE_AFTER))
    }
}

fn show_captions(world: &mut World) {
    let held = rows::held::<ui::Captions>(world);
    let delta = world.resource::<Time>().delta();
    let mut block = world.resource_mut::<CaptionBlock>();
    block.0.tick(delta, held);
    let next = ui::Captions {
        rows: block.0.labelled_lines(),
        more: block.0.more(),
    };
    for mut captions in world.query::<&mut ui::Captions>().iter_mut(world) {
        if *captions != next {
            *captions = next.clone();
        }
    }
}
