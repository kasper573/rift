use bevy_ecs::prelude::*;
use bevy_time::Time;

use crate::core::content::Content;
use crate::core::geometry::Radians;
use crate::core::interpolate::{Jumps, Predicted};
use crate::core::math::{Offset, Pos};
use crate::core::time::Seconds;
use crate::core::units::{Meters, TilePos};
use crate::systems::actor::render::RenderActor;
use crate::systems::actor::{Action, Actor};
use crate::systems::movement::{
    HeardMove, MoveRequest, MoveSeq, MoveTarget, Navigator, Path, Position, RenderPosition, gait,
    halt_walk, speed_of, stepping_off, walk,
};
use crate::systems::player::{commands_locked, session};
use crate::systems::stat;

/// Asks the server to walk the client's own character to `goal`, and sets off on screen at once
/// rather than a round trip later, when the server's walk shows up. A goal the walk already passes,
/// the tile it's stepping off included, carries the walk on to its end: asking again along the way
/// keeps a walk going, so asking for every tile of a line as the walk crosses them walks it exactly
/// as asking once for its end does.
pub fn request_walk(world: &mut World, goal: Pos<Meters>) {
    let Some(me) = predicted_character(world) else {
        return;
    };
    let free = !stat::is_dead(world, me) && !commands_locked(world, me);
    let content = world.resource::<Content>().clone();
    let navigator = Navigator::of_in(&content, world, me);
    let Some(mut stride) = world.get_mut::<Stride>(me) else {
        return;
    };
    let goal = stride.end_through(goal);
    stride.sent = MoveSeq(stride.sent.0 + 1);
    let seq = stride.sent;
    if free && let Some(navigator) = navigator {
        stride.head_for(&navigator, goal);
    }
    world.write_message(MoveRequest { pos: goal, seq });
}

/// The client's own character as it walks on screen. It runs ahead of the server, which hears of
/// each walk a trip over the network later, but it plans and walks the same way, so a walk the
/// client asked for looks on screen as it plays out on the server, only sooner. The server stays the
/// authority: once it has heard every request sent, the stride heads wherever the server's walk
/// goes, which is where the stride was going anyway unless the server sent the character elsewhere.
#[derive(Component)]
pub(super) struct Stride {
    at: Pos<Meters>,
    path: Path,
    target: Option<Pos<Meters>>,
    facing: Radians,
    /// The latest [`MoveRequest`] sent for this character, to compare with its [`HeardMove`].
    sent: MoveSeq,
    /// The server's count of the character's jumps when the stride last looked.
    jumps: Jumps,
}

pub(super) fn walk_ahead(world: &mut World) {
    let Some(me) = predicted_character(world) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let (Some(server), Some(navigator)) = (
        world.get::<Position>(me).cloned(),
        Navigator::of_in(&content, world, me),
    ) else {
        return;
    };
    let heard = world
        .get::<HeardMove>(me)
        .map_or(MoveSeq::default(), |heard| heard.0);
    let server_goal = world.get::<MoveTarget>(me).map(|target| target.pos);
    let alive = !stat::is_dead(world, me);
    let speed = speed_of(world, me);
    let dt = Seconds(world.resource::<Time>().delta_secs());
    let shown = world
        .get::<Actor>(me)
        .map(|actor| (actor.facing, actor.action));
    let Some(mut stride) = world.get_mut::<Stride>(me) else {
        return;
    };
    stride.track(&server);
    let heading = if alive {
        if heard >= stride.sent {
            stride.follow(&navigator, server_goal, server.pos);
        }
        stride.advance(speed * dt)
    } else {
        stride.snap(server.pos);
        None
    };
    let pose = stride.pose(heading, gait(speed), shown);
    let at = stride.at;
    if let Some(mut render) = world.get_mut::<RenderPosition>(me) {
        render.0 = at;
    }
    if let Some(mut render) = world.get_mut::<RenderActor>(me) {
        *render = pose;
    }
}

impl Stride {
    fn new(server: &Position, facing: Radians) -> Stride {
        Stride {
            at: server.pos,
            path: Path::default(),
            target: None,
            facing,
            sent: MoveSeq::default(),
            jumps: server.jumps,
        }
    }

    /// Where a walk through `goal` ends: at the stride's own end when `goal` lies on the way to it.
    fn end_through(&self, goal: Pos<Meters>) -> Pos<Meters> {
        let Some(end) = self.target else {
            return goal;
        };
        let cell = goal.cell();
        let underfoot = match self.path.next() {
            Some(next) if !self.at.on_center() => stepping_off(self.at, next),
            _ => self.at.cell(),
        };
        if cell == underfoot || self.path.tiles.contains(&cell) {
            end
        } else {
            goal
        }
    }

    /// Plans the walk to `goal` as the server does, returning whether it found a way there.
    fn head_for(&mut self, navigator: &Navigator, goal: Pos<Meters>) -> bool {
        match navigator.plan(self.at, Some(&self.path), goal) {
            Some(tiles) => {
                self.path = Path { tiles };
                self.target = Some(goal);
                true
            }
            None => {
                self.halt();
                false
            }
        }
    }

    fn halt(&mut self) {
        self.target = halt_walk(&mut self.at, &mut self.path);
    }

    /// Heads where the server's walk ends, or with none, where the server has the character stand.
    fn follow(
        &mut self,
        navigator: &Navigator,
        server_goal: Option<Pos<Meters>>,
        server: Pos<Meters>,
    ) {
        let goal = server_goal.unwrap_or(server);
        if self.target.unwrap_or(self.at) == goal {
            return;
        }
        let walked_there = server_goal.is_some() || server.on_center();
        if !walked_there || !self.head_for(navigator, goal) {
            self.snap(server);
        }
    }

    /// Shows the character where the server put it, should it have jumped: walking there would only
    /// show a walk that never happened.
    fn track(&mut self, server: &Position) {
        if std::mem::replace(&mut self.jumps, server.jumps) != server.jumps {
            self.snap(server.pos);
        }
    }

    fn snap(&mut self, to: Pos<Meters>) {
        self.at = to;
        self.path = Path::default();
        self.target = None;
    }

    fn advance(&mut self, distance: Meters) -> Option<Offset<Meters>> {
        let heading = walk(&mut self.at, &mut self.path, distance);
        if self.path.tiles.is_empty() {
            self.target = None;
        }
        heading
    }

    /// The stride's own walking pose, unless the server shows the character doing something walking
    /// can't: a swing or a death shows as soon as the server's word of it arrives.
    fn pose(
        &mut self,
        heading: Option<Offset<Meters>>,
        gait: Action,
        shown: Option<(Radians, Action)>,
    ) -> RenderActor {
        match (heading, shown) {
            (_, Some((facing, action @ (Action::Attack | Action::Dead)))) => {
                RenderActor { facing, action }
            }
            (Some(step), _) => {
                self.facing = Radians::of(step);
                RenderActor {
                    facing: self.facing,
                    action: gait,
                }
            }
            (None, shown) => {
                if let Some((facing, Action::Idle)) = shown {
                    self.facing = facing;
                }
                RenderActor {
                    facing: self.facing,
                    action: Action::Idle,
                }
            }
        }
    }
}

fn predicted_character(world: &mut World) -> Option<Entity> {
    let me = session::my_character(world)?.id();
    if !world.entity(me).contains::<Stride>() {
        let server = world.get::<Position>(me)?.clone();
        let actor = world.get::<Actor>(me)?;
        let pose = RenderActor {
            facing: actor.facing,
            action: actor.action,
        };
        world.entity_mut(me).insert((
            Stride::new(&server, pose.facing),
            Predicted,
            RenderPosition(server.pos),
            pose,
        ));
    }
    Some(me)
}
