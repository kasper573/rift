use bevy::prelude::*;

use super::rows::{self, NotificationRows, VoiceRows};
use super::{NotificationKind, NotificationSent};
use crate::core::time::Seconds;
use crate::systems::scene::Scene as GameScene;

const ALERTS: usize = 2;
const REPLACE_AFTER: Seconds = Seconds(3.0);

pub struct AlertPlugin;

impl Plugin for AlertPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AlertBlock>()
            .add_systems(Update, show_alerts.run_if(in_state(GameScene::Area)))
            .add_observer(close);
    }
}

pub fn raise(world: &mut World, notification_sent: NotificationSent) {
    let NotificationKind::Alert { label, .. } = &notification_sent.notification.kind else {
        return;
    };
    let label = label.clone();
    world
        .resource_mut::<AlertBlock>()
        .0
        .join(label, notification_sent);
}

pub fn shown(world: &World) -> NotificationRows {
    world.resource::<AlertBlock>().0.shown()
}

#[derive(Resource)]
struct AlertBlock(VoiceRows);

impl Default for AlertBlock {
    fn default() -> AlertBlock {
        AlertBlock(VoiceRows::new(ALERTS, REPLACE_AFTER))
    }
}

fn show_alerts(world: &mut World) {
    let held = rows::held::<ui::AlertBar>(world);
    let delta = world.resource::<Time>().delta();
    let mut block = world.resource_mut::<AlertBlock>();
    block.0.tick(delta, held);
    let next = ui::AlertBar {
        alerts: block.0.labelled_lines(),
        more: block.0.more(),
    };
    for mut bar in world.query::<&mut ui::AlertBar>().iter_mut(world) {
        if *bar != next {
            *bar = next.clone();
        }
    }
}

fn close(closed: On<ui::AlertClosed>, mut block: ResMut<AlertBlock>) {
    block.0.close(closed.key);
}
