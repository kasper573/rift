use std::time::Duration;

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_time::Time;
use bevy_ui::{ComputedNode, Display, FlexDirection, Node, Overflow, UiRect, UiTransform, Val};

use crate::components::rich_text::MotionPreference;
use crate::motion::{Motion, Timing, Transform2d};
use crate::opacity::Opacity;

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[require(Node, Motion, UiTransform, Opacity(0.0))]
pub struct Presence {
    pub shown: bool,
    pub enter: PresenceMove,
    pub exit: PresenceMove,
    collapses: bool,
    stage: Stage,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PresenceMove {
    pub away: Transform2d,
    pub timing: Timing,
}

#[derive(Component, Clone, Copy, Default, Debug)]
pub struct Leaving;

impl Presence {
    pub const fn new(enter: PresenceMove, exit: PresenceMove) -> Presence {
        Presence {
            shown: true,
            enter,
            exit,
            collapses: false,
            stage: Stage::Fresh,
        }
    }

    pub const fn hidden(self) -> Presence {
        Presence {
            shown: false,
            ..self
        }
    }

    pub const fn collapsing(self) -> Presence {
        Presence {
            collapses: true,
            ..self
        }
    }
}

impl PresenceMove {
    pub const fn new(away: Transform2d, timing: Timing) -> PresenceMove {
        PresenceMove { away, timing }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Fresh,
    Shown,
    Exiting {
        since: Duration,
        fold: Option<Fold>,
    },
    Hidden {
        display: Display,
        fold: Option<Fold>,
    },
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Fold {
    across: bool,
    extent: f32,
    gap: f32,
    was: (Val, Val, UiRect, Overflow),
}

#[derive(Component)]
pub(crate) struct Departs(Duration);

pub(crate) fn advance_leaving(
    time: Res<Time>,
    leaving: Query<(Entity, Option<&Departs>), With<Leaving>>,
    stayed: Query<Entity, (With<Departs>, Without<Leaving>)>,
    children: Query<&Children>,
    mut presences: Query<&mut Presence>,
    mut commands: Commands,
) {
    let now = time.elapsed();
    for entity in &stayed {
        commands.entity(entity).remove::<Departs>();
    }
    for (entity, departs) in &leaving {
        match departs {
            Some(Departs(at)) if now >= *at => commands.entity(entity).try_despawn(),
            Some(_) => {}
            None => {
                let mut longest = Duration::ZERO;
                let mut stack = vec![entity];
                while let Some(node) = stack.pop() {
                    match presences.get_mut(node) {
                        Ok(mut presence) => {
                            presence.shown = false;
                            longest = longest.max(presence.exit.timing.duration);
                        }
                        Err(_) => stack.extend(children.get(node).into_iter().flatten()),
                    }
                }
                commands.entity(entity).insert(Departs(now + longest));
            }
        }
    }
}

pub(crate) fn advance_presence(
    time: Res<Time>,
    preference: Res<MotionPreference>,
    mut presences: Query<(Entity, &mut Presence, &mut Motion, Option<&ChildOf>)>,
    mut nodes: Query<(&mut Node, &ComputedNode)>,
) {
    let now = time.elapsed();
    let pose = |movement: PresenceMove| match preference.reduced {
        true => Transform2d::IDENTITY,
        false => movement.away,
    };
    for (entity, mut presence, mut motion, parent) in &mut presences {
        let Presence {
            shown,
            enter,
            exit,
            collapses,
            stage,
        } = *presence;
        let fold = match (shown, stage, collapses, parent) {
            (false, Stage::Shown, true, Some(parent)) => {
                match (nodes.get(parent.parent()), nodes.get(entity)) {
                    (Ok((parent, _)), Ok((node, computed))) => {
                        Some(Fold::of(parent, node, computed))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let Ok((mut node, _)) = nodes.get_mut(entity) else {
            continue;
        };
        let next = match (shown, stage) {
            (true, stage) => {
                match stage {
                    Stage::Exiting { fold, .. } => unfold(&mut node, fold),
                    Stage::Hidden { display, fold } => {
                        node.display = display;
                        unfold(&mut node, fold);
                    }
                    Stage::Fresh | Stage::Shown => {}
                }
                motion.aim_opacity(0.0, 1.0, Some(enter.timing));
                motion.aim_transform(pose(enter), Transform2d::IDENTITY, Some(enter.timing));
                Stage::Shown
            }
            (false, Stage::Shown) => Stage::Exiting { since: now, fold },
            (false, Stage::Exiting { since, fold }) => {
                let elapsed = now.saturating_sub(since);
                let done = elapsed >= exit.timing.duration;
                if let Some(fold) = fold {
                    // Squared so the space closes behind the fade instead of clipping visible content.
                    fold.apply(&mut node, eased(exit.timing, elapsed).powi(2));
                }
                match done {
                    true => {
                        let display = node.display;
                        node.display = Display::None;
                        Stage::Hidden { display, fold }
                    }
                    false => stage,
                }
            }
            (false, Stage::Fresh) => {
                let display = node.display;
                node.display = Display::None;
                Stage::Hidden {
                    display,
                    fold: None,
                }
            }
            (false, Stage::Hidden { .. }) => stage,
        };
        match next {
            Stage::Exiting { .. } => {
                motion.aim_opacity(0.0, 0.0, Some(exit.timing));
                motion.aim_transform(pose(enter), pose(exit), Some(exit.timing));
            }
            Stage::Hidden { .. } if stage != next => {
                motion.aim_opacity(0.0, 0.0, None);
                motion.aim_transform(pose(enter), pose(enter), None);
            }
            _ => {}
        }
        if stage != next {
            presence.stage = next;
        }
    }
}

impl Fold {
    fn of(parent: &Node, node: &Node, computed: &ComputedNode) -> Fold {
        let across = matches!(
            parent.flex_direction,
            FlexDirection::Row | FlexDirection::RowReverse
        );
        let size = computed.size() * computed.inverse_scale_factor;
        let (extent, gap) = match across {
            true => (size.x, parent.column_gap),
            false => (size.y, parent.row_gap),
        };
        Fold {
            across,
            extent,
            gap: match gap {
                Val::Px(gap) => gap,
                _ => 0.0,
            },
            was: (node.width, node.height, node.margin, node.overflow),
        }
    }

    fn apply(self, node: &mut Node, folded: f32) {
        let extent = Val::Px(self.extent * (1.0 - folded));
        let pull = Val::Px(-self.gap * folded);
        node.overflow = Overflow::clip();
        match self.across {
            true => {
                node.width = extent;
                node.margin.right = pull;
            }
            false => {
                node.height = extent;
                node.margin.bottom = pull;
            }
        }
    }
}

fn unfold(node: &mut Node, fold: Option<Fold>) {
    if let Some(Fold {
        was: (width, height, margin, overflow),
        ..
    }) = fold
    {
        node.width = width;
        node.height = height;
        node.margin = margin;
        node.overflow = overflow;
    }
}

fn eased(timing: Timing, elapsed: Duration) -> f32 {
    let raw = elapsed.as_secs_f32() / timing.duration.as_secs_f32().max(f32::EPSILON);
    timing.easing.eval(raw.clamp(0.0, 1.0))
}
