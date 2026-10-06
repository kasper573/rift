use bevy::prelude::*;

use super::rows::{self, NotificationRows, VoiceRows};
use super::{NotificationKind, NotificationSent};
use crate::core::time::Seconds;
use crate::systems::scene::Scene as GameScene;

const ROWS: usize = 2;
const REPLACE_AFTER: Seconds = Seconds(2.0);

pub struct MilestonePlugin;

impl Plugin for MilestonePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MilestoneBlock>()
            .add_systems(Update, show_milestones.run_if(in_state(GameScene::Area)));
    }
}

pub fn reach(world: &mut World, notification_sent: NotificationSent) {
    let NotificationKind::Milestone { label, .. } = &notification_sent.notification.kind else {
        return;
    };
    let label = label.clone();
    world
        .resource_mut::<MilestoneBlock>()
        .0
        .join(label, notification_sent);
}

pub fn shown(world: &World) -> NotificationRows {
    world.resource::<MilestoneBlock>().0.shown()
}

#[derive(Resource)]
struct MilestoneBlock(VoiceRows);

impl Default for MilestoneBlock {
    fn default() -> MilestoneBlock {
        MilestoneBlock(VoiceRows::new(ROWS, REPLACE_AFTER))
    }
}

fn show_milestones(world: &mut World) {
    let held = rows::held::<ui::Milestones>(world);
    let delta = world.resource::<Time>().delta();
    let mut block = world.resource_mut::<MilestoneBlock>();
    block.0.tick(delta, held);
    let next = ui::Milestones {
        rows: block.0.labelled_lines(),
        more: block.0.more(),
    };
    for mut milestones in world.query::<&mut ui::Milestones>().iter_mut(world) {
        if *milestones != next {
            *milestones = next.clone();
        }
    }
}
