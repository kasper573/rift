use std::path::Path;
use std::time::{Duration, SystemTime};

use bevy_ecs::entity::Entity;
use bevy_input::keyboard::KeyCode;
use bevy_input::mouse::MouseButton;
use engine::core::math::{Offset, Pos, Rect, WindowPx};
use engine::core::units::{MeterPos, Meters};
use engine::systems::area;
use engine::systems::input::gestures::ClickClaim;
use engine::systems::input::map::{InputAction, InputBinding, InputMap, KeyModifiers};
use engine::systems::movement::{NavMesh, SPACING};
use engine::systems::probe::{ProbeBody, ProbeShop, ProbeSnapshot, ProbeStage, ProbeUiElement};

use crate::content::{content, hunting_ground, lattice, marker_at};
use crate::desktop::{BUDGET, Button, Deadline, Desktop, WaitBudget};
use crate::recording::Footage;
use crate::seen::{
    UiTarget, aiming, area_name, at_rest, beside_anyone, clickable_point, in_warp, in_world,
    me_at_rest, object_named, open, passed_by, taken, townsperson, warp_named,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    // As fast as the game takes input.
    Brisk,
    // As a viewer can follow: the pointer glides, keys come a beat apart, and words stay up long
    // enough to read.
    Viewer,
}

/// Someone playing on a desktop: they act through the game's own input bindings, and judge every
/// click by the frame the game saw it in.
pub struct Player {
    desktop: Desktop,
    pace: Pace,
}

pub const LONG: WaitBudget = WaitBudget {
    time: Duration::from_secs(120),
    frames: 3600,
};

const NUDGE: WaitBudget = WaitBudget {
    time: Duration::from_secs(2),
    frames: 120,
};

impl Player {
    pub fn new(desktop: Desktop, pace: Pace) -> Player {
        Player { desktop, pace }
    }

    pub fn snapshot(&mut self) -> ProbeSnapshot {
        self.desktop.snapshot()
    }

    pub fn wait_for<T>(&mut self, what: &str, found: impl FnMut(&ProbeSnapshot) -> Option<T>) -> T {
        self.desktop.wait_for(what, BUDGET, found)
    }

    pub fn wait_within<T>(
        &mut self,
        what: &str,
        budget: WaitBudget,
        found: impl FnMut(&ProbeSnapshot) -> Option<T>,
    ) -> T {
        self.desktop.wait_for(what, budget, found)
    }

    /// As `wait_for`, but running out is an answer rather than a failure: for what may not come.
    pub fn try_wait_for<T>(
        &mut self,
        budget: WaitBudget,
        found: impl FnMut(&ProbeSnapshot) -> Option<T>,
    ) -> Option<T> {
        self.desktop.try_wait_for(budget, found)
    }

    pub fn wait_until(
        &mut self,
        what: &str,
        mut done: impl FnMut(&ProbeSnapshot) -> bool,
    ) -> ProbeSnapshot {
        self.wait_for(what, |snapshot| done(snapshot).then(|| snapshot.clone()))
    }

    pub fn act(&mut self, action: InputAction) -> ProbeSnapshot {
        let gesture = bindings(action)
            .iter()
            .find_map(|binding| match binding {
                InputBinding::Key(gesture) => Some(*gesture),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no key is bound to {action:?}"));
        if self.pace == Pace::Viewer {
            std::thread::sleep(KEY_BEAT);
        }
        self.with_held(gesture.modifiers, |player| {
            player.desktop.key(&keysym(gesture.key))
        })
    }

    pub fn with_held<T>(&mut self, held: KeyModifiers, during: impl FnOnce(&mut Player) -> T) -> T {
        let keys = modifier_keys(held);
        for key in &keys {
            self.desktop.key_down(key);
        }
        let done = during(self);
        for key in keys.iter().rev() {
            self.desktop.key_up(key);
        }
        done
    }

    /// Returns the first frame the game saw the pointer there. On camera it glides over a few
    /// frames, as a hand would.
    pub fn point_at(&mut self, to: Pos<WindowPx>) -> ProbeSnapshot {
        let from = self.desktop.cursor().unwrap_or(to);
        let steps = match self.pace {
            Pace::Viewer if from.distance_to(to) > 4.0 => GLIDE_STEPS,
            _ => 1,
        };
        let mut seen = None;
        for step in 1..=steps {
            seen = Some(
                self.desktop
                    .move_pointer(from.lerp(to, step as f32 / steps as f32)),
            );
        }
        seen.expect("at least one step")
    }

    pub fn sweep_pointer(&mut self, through: &[Pos<WindowPx>]) -> ProbeSnapshot {
        self.desktop.sweep_pointer(through)
    }

    pub fn point_at_tile(&mut self, tile: Pos<Meters>) -> ProbeSnapshot {
        let seen = self.snapshot();
        let Some(at) = clickable_point(&seen, tile) else {
            self.desktop.fail(&format!("{tile:?} is out of view"));
        };
        self.point_at(at)
    }

    /// Whatever the action's pointer binding is, where the pointer is.
    pub fn act_here(&mut self, action: InputAction) -> ProbeSnapshot {
        let (button, times, held) = pointer_binding(action);
        self.with_held(held, |player| {
            player.desktop.click(button_of(button), times)
        })
    }

    /// The interface's drag of `ui`, held while `during` plays and let go after it.
    pub fn dragging_ui<T>(
        &mut self,
        ui: impl Into<UiTarget>,
        during: impl FnOnce(&mut Player) -> T,
    ) -> T {
        self.aim_at_ui(&ui.into(), Aim::Click);
        self.desktop.button_down(Button::Left);
        let done = during(self);
        self.desktop.button_up(Button::Left);
        done
    }

    pub fn click(&mut self) -> ProbeSnapshot {
        self.act_here(InputAction::Interact)
    }

    pub fn find_ui(&mut self, ui: impl Into<UiTarget>) -> ProbeUiElement {
        let ui = ui.into();
        self.wait_for(
            &format!("the interface to show {}", ui.describe()),
            |snapshot| ui.find(snapshot).cloned(),
        )
    }

    pub fn hover_ui(&mut self, ui: impl Into<UiTarget>) -> ProbeSnapshot {
        self.aim_at_ui(&ui.into(), Aim::Hover)
    }

    pub fn click_ui(&mut self, ui: impl Into<UiTarget>) -> ProbeSnapshot {
        self.act_on_ui(InputAction::Select, ui)
    }

    pub fn act_on_ui(&mut self, action: InputAction, ui: impl Into<UiTarget>) -> ProbeSnapshot {
        self.aim_at_ui(&ui.into(), Aim::Click);
        self.act_here(action)
    }

    /// Pressed, moved over frames so the game sees the motion, and released.
    pub fn drag_ui(&mut self, ui: impl Into<UiTarget>, by: Offset<WindowPx>) -> ProbeSnapshot {
        let from = self.aim_at_ui(&ui.into(), Aim::Click);
        let from = from.input.cursor.expect("the pointer is over the window");
        self.drag(Button::Left, KeyModifiers::NONE, from + by)
    }

    pub fn drag_from(&mut self, from: Pos<WindowPx>, by: Offset<WindowPx>) -> ProbeSnapshot {
        self.point_at(from);
        self.drag(Button::Left, KeyModifiers::NONE, from + by)
    }

    pub fn carry_onto(
        &mut self,
        item: impl Into<UiTarget>,
        onto: impl Into<UiTarget>,
    ) -> ProbeSnapshot {
        let (button, held) = carry_binding();
        self.aim_at_ui(&item.into(), Aim::Click);
        let target = self.find_ui(onto);
        self.drag(button_of(button), held, target.rect.center())
    }

    pub fn carrying<T>(
        &mut self,
        item: impl Into<UiTarget>,
        during: impl FnOnce(&mut Player) -> T,
    ) -> T {
        let (button, held) = carry_binding();
        self.aim_at_ui(&item.into(), Aim::Click);
        self.with_held(held, |player| {
            player.desktop.button_down(button_of(button));
            let done = during(player);
            player.desktop.button_up(button_of(button));
            done
        })
    }

    /// A tab sits on the row of tabs that "All" heads; the same title can show in the records below
    /// it.
    pub fn open_history_tab(&mut self, title: &str) {
        self.click_ui(UiTarget::button(title));
        self.wait_until(&format!("the {title} tab"), |snapshot| {
            snapshot.history.tab.as_deref() == Some(title)
        });
    }

    /// Turns the wheel over `within` a line at a time until `shown` scrolls into sight; the other
    /// way, should it not show that way.
    pub fn scroll_until(
        &mut self,
        within: impl Into<UiTarget>,
        shown: impl Into<UiTarget>,
    ) -> ProbeUiElement {
        let (within, shown) = (within.into(), shown.into());
        self.aim_at_ui(&within, Aim::Hover);
        for step in 0..2 * SCROLLS {
            if let Some(found) = shown.find(&self.settled()) {
                return found.clone();
            }
            self.desktop.wheel(if step < SCROLLS { 1 } else { -1 });
        }
        self.desktop
            .fail(&format!("{} never scrolled into sight", shown.describe()));
    }

    /// The first frame in which the interface no longer moves: windows ease open, and lists glide
    /// as they scroll. What rides on the world moves with it, and never settles while anyone walks.
    pub fn settled(&mut self) -> ProbeSnapshot {
        let mut last = Vec::new();
        self.wait_for("the interface to settle", |snapshot| {
            let interface: Vec<ProbeUiElement> = snapshot
                .ui
                .iter()
                .filter(|element| !element.in_world)
                .cloned()
                .collect();
            let settled = interface == last;
            last = interface;
            settled.then(|| snapshot.clone())
        })
    }

    pub fn type_text(&mut self, text: &str, landed: impl FnMut(&ProbeSnapshot) -> bool) {
        let delay = match self.pace {
            Pace::Viewer => TYPING,
            Pace::Brisk => Duration::from_millis(1),
        };
        self.desktop.type_text(text, delay, landed);
    }

    /// Walks there and comes to rest, and returns where: anyone standing on the tile stops the
    /// player short.
    pub fn walk_to(&mut self, tile: Pos<Meters>) -> Pos<Meters> {
        self.walk_toward(|_| Some(tile))
    }

    /// As `walk_to`, for a goal re-read every frame, as something that moves.
    pub fn walk_toward(
        &mut self,
        goal: impl Fn(&ProbeSnapshot) -> Option<Pos<Meters>>,
    ) -> Pos<Meters> {
        self.travel_to(&goal, |snapshot| {
            goal(snapshot).is_some_and(|goal| hemmed_in(snapshot, goal))
        });
        self.come_to_rest()
    }

    /// The ground the player can walk to nearest the spot that nobody stands on, clear of warps.
    pub fn walk_near(&mut self, spot: Pos<Meters>) -> Pos<Meters> {
        let seen = self.in_world();
        let mesh = mesh_of(&seen);
        let me = seen.me.as_ref().expect("the player").at;
        let ground = mesh
            .nearest_where(spot, SPACING, |tile| {
                mesh.connects(me, tile)
                    && !in_warp(&seen, tile)
                    && !taken(&seen, tile)
                    && !beside_anyone(&seen, tile)
            })
            .expect("ground the player can walk to");
        self.walk_to(ground)
    }

    pub fn walk_by(&mut self, offset: Offset<Meters>) -> Pos<Meters> {
        let me = self.in_world().me.expect("the player").at;
        self.walk_near(me + offset)
    }

    /// Into a marked spot of the area: a named object, or a townsperson's home.
    pub fn walk_into(&mut self, marker: &str, reach: Meters) -> Pos<Meters> {
        let seen = self.in_world();
        let area = area_name(&seen).expect("the player is in an area");
        let at = marker_at(area, marker);
        let inside = |me: Pos<Meters>| (me.x - at.x).abs().max((me.y - at.y).abs()) <= reach.0;
        self.travel_to(
            |_| Some(at),
            |snapshot| snapshot.me.as_ref().is_some_and(|me| inside(me.at)),
        );
        self.come_to_rest()
    }

    /// Through the warp to wherever it leads. A warp takes whoever steps into it, so a player
    /// standing in one steps off before stepping on.
    pub fn cross(&mut self, warp: &str) -> ProbeSnapshot {
        let seen = self.in_world();
        let gate = warp_named(&seen, warp).clone();
        if gate.rect.contains(seen.me.as_ref().expect("the player").at) {
            let off = mesh_of(&seen)
                .nearest_where(gate.at, SPACING, |tile| {
                    open(&seen, tile) && !in_warp(&seen, tile)
                })
                .expect("open ground beside the warp");
            self.walk_to(off);
        }
        self.travel_to(|_| Some(gate.at), |snapshot| snapshot.area == Some(gate.to));
        self.wait_for(&format!("the crossing through {warp}"), |snapshot| {
            (snapshot.area == Some(gate.to) && in_world(snapshot)).then(|| snapshot.clone())
        })
    }

    /// Heads for the goal and ends once the player stands there, or as soon as `arrived` holds.
    /// Only what's in view can be clicked, so a far goal is approached along the game's own path to
    /// it, one click on the farthest open ground in view at a time.
    pub fn travel_to(
        &mut self,
        goal: impl Fn(&ProbeSnapshot) -> Option<Pos<Meters>>,
        arrived: impl Fn(&ProbeSnapshot) -> bool,
    ) {
        let deadline = Deadline::from(&self.desktop, LONG);
        loop {
            let seen = self.snapshot();
            if arrived(&seen) {
                return;
            }
            if deadline.passed(&self.desktop) {
                self.desktop.give_up("the way there");
            }
            if seen.stage.is_some() {
                self.leave_conversation();
                continue;
            }
            let (true, Some(me), Some(goal)) = (in_world(&seen), seen.me.as_ref(), goal(&seen))
            else {
                continue;
            };
            let mesh = mesh_of(&seen);
            let target = reachable_near(mesh, me.at, goal).expect("ground the player can walk to");
            if at_rest(me) && me.at.distance(target) < CLICK_SLACK {
                return;
            }
            let Some(path) = mesh.route(me.at, target) else {
                continue;
            };
            let hop = path
                .iter()
                .rev()
                .copied()
                .find(|tile| {
                    open(&seen, *tile)
                        && if *tile == target {
                            !passed_by(&seen, *tile)
                        } else {
                            !beside_anyone(&seen, *tile)
                        }
                        && (!in_warp(&seen, *tile) || *tile == target)
                })
                .or_else(|| detour(&seen, mesh, target, length(me.at, &path)));
            // Those standing in the way move on, or `arrived` says how close is close enough.
            let Some(hop) = hop else {
                continue;
            };
            let onto_goal = hop == target;
            let wanted = |claim: &ClickClaim| match claim {
                ClickClaim::Walk(tile) if onto_goal => tile.distance(target) < CLICK_SLACK,
                ClickClaim::Walk(tile) => {
                    tile.distance(hop) < CLICK_SLACK
                        || path
                            .iter()
                            .any(|corner| corner.distance(*tile) < CLICK_SLACK)
                }
                _ => false,
            };
            let at = clickable_point(&seen, hop).expect("open ground is in view");
            if !wanted(&self.point_at(at).claim) || !wanted(&self.click().claim) {
                continue;
            }
            // On the way the next click goes out as the player nears this one; onto the goal, once
            // the walk ends.
            self.set_off(&arrived);
            self.wait_until("the walk toward the goal", |snapshot| {
                arrived(snapshot)
                    || snapshot.stage.is_some()
                    || me_at_rest(snapshot)
                    || !onto_goal
                        && snapshot
                            .me
                            .as_ref()
                            .is_some_and(|me| me.at.distance(hop) < NEAR)
            });
        }
    }

    /// Heads for where `npc` roams, and ends once `done` holds.
    pub fn look_out(&mut self, npc: &str, done: impl Fn(&ProbeSnapshot) -> bool) -> Pos<Meters> {
        let area = area_name(&self.snapshot()).expect("an area");
        let ground = hunting_ground(area, npc);
        self.look_for(&format!("{npc} in sight"), ground, done);
        ground
    }

    /// Others show only once near enough: the player heads for where they are, and finds them on
    /// the way or there.
    pub fn look_for(
        &mut self,
        what: &str,
        place: Pos<Meters>,
        found: impl Fn(&ProbeSnapshot) -> bool,
    ) -> ProbeSnapshot {
        self.travel_to(|_| Some(place), &found);
        self.wait_until(what, found)
    }

    /// In the world and free to act: a newcomer's commands are held for a moment after arriving.
    pub fn in_world(&mut self) -> ProbeSnapshot {
        self.wait_until("the player in the world", |snapshot| {
            in_world(snapshot) && snapshot.me.as_ref().is_some_and(|me| !me.locked)
        })
    }

    pub fn come_to_rest(&mut self) -> Pos<Meters> {
        self.wait_for("the player to come to rest", |snapshot| {
            snapshot
                .me
                .as_ref()
                .filter(|me| at_rest(me))
                .map(|me| me.at)
        })
    }

    // Walks up to the target, and clicks it until `done` shows what the click did.
    fn engage<T>(
        &mut self,
        what: &str,
        locate: impl Fn(&ProbeSnapshot) -> Option<(Entity, Pos<Meters>, Pos<Meters>)>,
        done: impl Fn(&ProbeSnapshot) -> Option<T>,
    ) -> T {
        let found = self.reach(
            what,
            LONG,
            locate,
            |seen, target| seen.claim == ClickClaim::Interact(target),
            done,
        );
        found.unwrap_or_else(|| self.desktop.give_up(what))
    }

    pub fn talk_to(&mut self, npc: &str) -> ProbeStage {
        self.engage(
            &format!("the conversation with {npc}"),
            |snapshot| townsperson(snapshot, npc).map(|body| (body.id, body.at, body.aim)),
            |snapshot| {
                let with = townsperson(snapshot, npc)?.id;
                stage_with(snapshot, with)
            },
        )
    }

    pub fn interact_with(&mut self, object: &str) -> ProbeStage {
        self.engage(
            &format!("using {object}"),
            |snapshot| object_named(snapshot, object).map(|seen| (seen.id, seen.at, seen.aim)),
            |snapshot| {
                let with = object_named(snapshot, object)?.id;
                stage_with(snapshot, with)
            },
        )
    }

    pub fn shop_at(&mut self, object: &str) -> ProbeShop {
        self.engage(
            &format!("the shop at {object}"),
            |snapshot| object_named(snapshot, object).map(|seen| (seen.id, seen.at, seen.aim)),
            |snapshot| snapshot.shop.clone(),
        )
    }

    /// Arms the skill in the slot and points at the tile, ready to cast.
    pub fn aim_skill(&mut self, slot: InputAction, at: Pos<Meters>) {
        self.act(slot);
        self.wait_until("the skill armed", aiming);
        self.point_at_tile(at);
    }

    /// Casts the aimed skill where the pointer rests, until `done` shows the cast landed.
    pub fn cast(&mut self, done: impl Fn(&ProbeSnapshot) -> bool) {
        self.click();
        self.wait_until("the cast landed", done);
    }

    pub fn cast_at(
        &mut self,
        slot: InputAction,
        at: Pos<Meters>,
        done: impl Fn(&ProbeSnapshot) -> bool,
    ) {
        self.aim_skill(slot, at);
        self.cast(done);
    }

    /// Casts the skill in the slot on the actor `pick` names, clicking only once the pointer rests
    /// on that actor, since it may wander while the pointer glides; a click that still misses arms
    /// the skill again.
    pub fn cast_on(
        &mut self,
        slot: InputAction,
        pick: impl Fn(&ProbeSnapshot) -> Option<Entity>,
        done: impl Fn(&ProbeSnapshot) -> bool,
    ) {
        let deadline = Deadline::from(&self.desktop, LONG);
        loop {
            let seen = self.snapshot();
            if done(&seen) {
                return;
            }
            if deadline.passed(&self.desktop) {
                self.desktop.give_up("the cast on its target");
            }
            if !aiming(&seen) {
                self.act(slot);
                self.wait_until("the skill armed", aiming);
            }
            let Some(target) = pick(&seen) else {
                continue;
            };
            let Some(body) = seen.actors.iter().find(|actor| actor.id == target) else {
                continue;
            };
            let Some(point) = clickable_point(&seen, body.aim) else {
                continue;
            };
            let pointed = self.point_at(point);
            if !matches!(pointed.claim, ClickClaim::Aim(aimed) if aimed == target) {
                continue;
            }
            self.click();
            if self
                .try_wait_for(BUDGET, |snapshot| done(snapshot).then_some(()))
                .is_some()
            {
                return;
            }
        }
    }

    /// Chases the nearest of `foes` and swings at whichever of them the click lands on, until
    /// `done`: monsters crowd, and the one in front takes the click.
    pub fn fight(
        &mut self,
        foes: impl Fn(&ProbeSnapshot, &ProbeBody) -> bool,
        done: impl Fn(&ProbeSnapshot) -> bool,
    ) {
        let standing =
            |snapshot: &ProbeSnapshot, body: &ProbeBody| body.health > 0.0 && foes(snapshot, body);
        let fought = self.reach(
            "the fight",
            LONG,
            |snapshot| {
                let me = snapshot.me.as_ref()?.at;
                snapshot
                    .actors
                    .iter()
                    .filter(|actor| standing(snapshot, actor))
                    .min_by(|a, b| a.at.distance_to(me).total_cmp(&b.at.distance_to(me)))
                    .map(|actor| (actor.id, actor.at, actor.aim))
            },
            |seen, _| match seen.claim {
                ClickClaim::Attack(struck) => seen
                    .actors
                    .iter()
                    .any(|actor| actor.id == struck && standing(seen, actor)),
                _ => false,
            },
            |snapshot| done(snapshot).then_some(()),
        );
        if fought.is_none() {
            self.desktop.give_up("the fight");
        }
    }

    /// Whatever stands over the item takes a click instead, so it is clicked again as the way
    /// clears.
    pub fn pick_up(&mut self, item: Entity, budget: WaitBudget) {
        let what = "the pick-up";
        let lying = |snapshot: &ProbeSnapshot| {
            snapshot
                .items
                .iter()
                .find(|lying| lying.id == item)
                .map(|lying| (lying.id, lying.at, lying.at))
        };
        let takes = |seen: &ProbeSnapshot, target| seen.claim == ClickClaim::Pickup(target);
        let picked = self.reach(what, budget, lying, takes, |snapshot| {
            lying(snapshot).is_none().then_some(())
        });
        if picked.is_none() {
            self.desktop.give_up(what);
        }
    }

    /// Reads on line by line until the choices show. On camera each line, and then the choices,
    /// stay up long enough to read; the typewriter has already shown the line for as long as it
    /// took to type.
    pub fn read_to_choices(&mut self) -> ProbeStage {
        loop {
            let began = SystemTime::now();
            let stage = self.wait_for("a line to finish typing", |snapshot| {
                snapshot.stage.clone().filter(|stage| !stage.typing)
            });
            let last = stage.line + 1 >= stage.lines;
            if self.pace == Pace::Viewer {
                let mut read = reading(stage.text.as_deref().unwrap_or_default());
                if last {
                    let labels: Vec<&str> = stage
                        .choices
                        .iter()
                        .map(|choice| choice.label.as_str())
                        .collect();
                    read += reading(&labels.join(" "));
                }
                self.linger_until(began + read);
            }
            if last {
                return stage;
            }
            self.act(InputAction::Advance);
            self.wait_until("the line to advance", |snapshot| {
                snapshot
                    .stage
                    .as_ref()
                    .is_none_or(|now| now.line != stage.line || now.node != stage.node)
            });
        }
    }

    pub fn on_stage(&mut self, node: &str) -> ProbeStage {
        self.wait_for(&format!("{node} on stage"), |snapshot| {
            snapshot.stage.clone().filter(|stage| stage.node == node)
        })
    }

    pub fn pick(&mut self, label: &str) {
        self.aim_at_choice(label);
        self.act_here(InputAction::Select);
    }

    pub fn hover_choice(&mut self, label: &str) {
        self.aim_at_choice(label);
    }

    /// A walk click can land on a townsperson who strolled under it, and leaving one conversation
    /// can open another that someone held until the player was free.
    pub fn leave_conversation(&mut self) {
        let deadline = Deadline::from(&self.desktop, BUDGET);
        loop {
            let Some(before) = self.snapshot().stage else {
                return;
            };
            // Presses Dismiss again whenever a press went unanswered, since one can land in a frame
            // the stage is still settling and be lost.
            self.act(InputAction::Dismiss);
            let moved_on = self.try_wait_for(NUDGE, |snapshot| {
                snapshot
                    .stage
                    .as_ref()
                    .is_none_or(|stage| stage.with != before.with || stage.node != before.node)
                    .then_some(())
            });
            if moved_on.is_none() && deadline.passed(&self.desktop) {
                self.desktop.give_up("the conversation to end");
            }
        }
    }

    pub fn conversation_over(&mut self) {
        self.wait_until("the conversation to end", |snapshot| {
            snapshot.stage.is_none()
        });
    }

    pub fn answer_first_choices(&mut self) {
        for _ in 0..FIRST_CHOICES {
            let Some(before) = self.snapshot().stage else {
                return;
            };
            self.read_to_choices();
            self.act(InputAction::Choice1);
            self.wait_until("the conversation to move on", |snapshot| {
                snapshot
                    .stage
                    .as_ref()
                    .is_none_or(|now| now.node != before.node)
            });
        }
        self.conversation_over();
    }

    /// Lingers on what's on screen until then, a frame at a time, so every observer sees each
    /// frame.
    pub fn linger_until(&mut self, until: SystemTime) {
        while until.duration_since(SystemTime::now()).is_ok() {
            self.snapshot();
        }
    }

    pub fn observe(&mut self, observer: impl FnMut(&ProbeSnapshot) + 'static) {
        self.desktop.observe(observer);
    }

    pub fn stop(&mut self) -> Option<Footage> {
        self.desktop.stop()
    }

    pub fn folder(&self) -> &Path {
        self.desktop.folder()
    }

    // The server answers a click a round trip later: the player sets off, or it's already done.
    fn set_off(&mut self, done: &impl Fn(&ProbeSnapshot) -> bool) -> bool {
        self.desktop
            .try_wait_for(SET_OFF, |snapshot| {
                (done(snapshot) || snapshot.stage.is_some() || !me_at_rest(snapshot)).then_some(())
            })
            .is_some()
    }

    // Clicks it, re-read every frame, until `done`: the target moves, and others pass in front of
    // it, so each try aims anew. Gives up when the budget runs out.
    fn reach<T>(
        &mut self,
        what: &str,
        budget: WaitBudget,
        locate: impl Fn(&ProbeSnapshot) -> Option<(Entity, Pos<Meters>, Pos<Meters>)>,
        takes: impl Fn(&ProbeSnapshot, Entity) -> bool,
        done: impl Fn(&ProbeSnapshot) -> Option<T>,
    ) -> Option<T> {
        let deadline = Deadline::from(&self.desktop, budget);
        loop {
            let seen = self.snapshot();
            if let Some(found) = done(&seen) {
                return Some(found);
            }
            if deadline.passed(&self.desktop) {
                return None;
            }
            if seen.stage.is_some() {
                self.leave_conversation();
                continue;
            }
            let Some((target, _, aim)) = locate(&seen) else {
                continue;
            };
            let Some(at) = clickable_point(&seen, aim) else {
                self.travel_to(
                    |snapshot| locate(snapshot).map(|(_, at, _)| at),
                    |snapshot| {
                        done(snapshot).is_some()
                            || snapshot.stage.is_some()
                            || locate(snapshot)
                                .is_some_and(|(_, _, aim)| clickable_point(snapshot, aim).is_some())
                    },
                );
                continue;
            };
            let fair = |seen: &ProbeSnapshot| takes(seen, target) || in_the_way(seen, target);
            if !fair(&self.point_at(at)) || !fair(&self.click()) {
                continue;
            }
            // Approaching takes a while: done once it lands, or a fresh try once the player stands
            // still again (the target slipped away, or a swing ended).
            if self.set_off(&|snapshot| done(snapshot).is_some()) {
                self.wait_within(what, budget, |snapshot| {
                    (done(snapshot).is_some() || snapshot.stage.is_some() || me_at_rest(snapshot))
                        .then_some(())
                });
            }
        }
    }

    // Re-aimed until it is still under the pointer in the frame the game saw it there: windows ease
    // open, and lists shift as they fill. Clicks pass through some of the interface, so only the
    // rest can be clicked; all of it can be hovered.
    fn aim_at_ui(&mut self, ui: &UiTarget, aim: Aim) -> ProbeSnapshot {
        let found = |snapshot: &ProbeSnapshot| {
            snapshot
                .ui
                .iter()
                .filter(|element| aim == Aim::Hover || !element.click_through)
                .find(|element| ui.matches(element, snapshot))
                .map(|element| element.rect)
        };
        self.aim_at(
            &format!("the interface to show {}", ui.describe()),
            aim,
            found,
        )
    }

    // Choice labels can show elsewhere too (a quest title on the tracker), so only the choice row
    // counts.
    fn aim_at_choice(&mut self, label: &str) -> ProbeSnapshot {
        let row = |snapshot: &ProbeSnapshot| {
            snapshot
                .stage
                .as_ref()?
                .choices
                .iter()
                .find(|choice| choice.label == label)?
                .rect
        };
        self.aim_at(&format!("the choice {label:?}"), Aim::Click, row)
    }

    fn aim_at(
        &mut self,
        what: &str,
        aim: Aim,
        find: impl Fn(&ProbeSnapshot) -> Option<Rect<WindowPx>>,
    ) -> ProbeSnapshot {
        for _ in 0..AIM_ATTEMPTS {
            let target = self.wait_for(what, &find);
            let seen = self.point_at(target.center());
            let still = seen
                .input
                .cursor
                .is_some_and(|at| find(&seen).is_some_and(|rect| rect.contains(at)));
            if still && (aim == Aim::Hover || seen.claim == ClickClaim::Interface) {
                return seen;
            }
        }
        self.desktop
            .fail(&format!("{what} never held still under the pointer"))
    }

    fn drag(&mut self, button: Button, held: KeyModifiers, to: Pos<WindowPx>) -> ProbeSnapshot {
        let from = self.desktop.cursor().expect("the pointer is on the window");
        self.with_held(held, |player| {
            player.desktop.button_down(button);
            for step in 1..=DRAG_STEPS {
                player
                    .desktop
                    .move_pointer(from.lerp(to, step as f32 / DRAG_STEPS as f32));
            }
            player.desktop.button_up(button)
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Aim {
    Click,
    Hover,
}

const GLIDE_STEPS: u32 = 8;
const DRAG_STEPS: u32 = 8;
const KEY_BEAT: Duration = Duration::from_millis(250);
const TYPING: Duration = Duration::from_millis(45);
// An element that keeps moving out from under the pointer is chased this many times before giving
// up.
const AIM_ATTEMPTS: u32 = 60;
const NEAR: Meters = Meters(1.5);
/// How far a click may land from the ground point it aimed at, through the window's pixels.
const CLICK_SLACK: Meters = Meters(0.25);
const SCROLLS: u32 = 12;
const FIRST_CHOICES: u32 = 3;
const SET_OFF: WaitBudget = WaitBudget {
    time: Duration::from_secs(2),
    frames: 120,
};

/// About four words a second, the pace of subtitles, with a moment to find the text first.
pub fn reading(text: &str) -> Duration {
    Duration::from_millis(800 + 250 * text.split_whitespace().count() as u64)
}

fn stage_with(snapshot: &ProbeSnapshot, with: Entity) -> Option<ProbeStage> {
    snapshot
        .stage
        .clone()
        .filter(|stage| stage.with == Some(with))
}

fn bindings(action: InputAction) -> Vec<InputBinding> {
    InputMap::new(content()).bindings(action).to_vec()
}

fn mesh_of(snapshot: &ProbeSnapshot) -> &'static NavMesh {
    &area::load(content(), snapshot.area.expect("the player is in an area")).mesh
}

/// How far a walk from `from` along `corners` goes.
fn length(from: Pos<Meters>, corners: &[Pos<Meters>]) -> Meters {
    let mut at = from;
    let mut total = 0.0;
    for &corner in corners {
        total += at.distance(corner).0;
        at = corner;
    }
    Meters(total)
}

// At rest, with someone standing on every step left of the way: as close as they let the player
// come.
fn hemmed_in(snapshot: &ProbeSnapshot, goal: Pos<Meters>) -> bool {
    let Some(me) = snapshot.me.as_ref().filter(|me| at_rest(me)) else {
        return false;
    };
    let mesh = mesh_of(snapshot);
    reachable_near(mesh, me.at, goal)
        .and_then(|target| mesh.route(me.at, target))
        .is_some_and(|path| path.iter().all(|corner| taken(snapshot, *corner)))
}

// With everything in view along the way taken by those standing on it, the open ground in view with
// the least of the way left, should it leave less than `left`: a walk around them.
fn detour(
    snapshot: &ProbeSnapshot,
    mesh: &NavMesh,
    target: Pos<Meters>,
    left: Meters,
) -> Option<Pos<Meters>> {
    lattice(mesh)
        .filter(|tile| {
            open(snapshot, *tile) && !beside_anyone(snapshot, *tile) && !in_warp(snapshot, *tile)
        })
        .filter_map(|tile| Some((tile, length(tile, &mesh.route(tile, target)?))))
        .filter(|(_, way)| *way < left)
        .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
        .map(|(tile, _)| tile)
}

// An enemy in front of the target takes the click, as it would take a player's: the way to the
// target leads through it.
fn in_the_way(seen: &ProbeSnapshot, target: Entity) -> bool {
    matches!(seen.claim, ClickClaim::Attack(other) | ClickClaim::Pickup(other) if other != target)
}

// The goal itself, or for a goal off the walkways (someone behind a counter), the ground the player
// can walk to nearest it.
fn reachable_near(mesh: &NavMesh, from: Pos<Meters>, goal: Pos<Meters>) -> Option<Pos<Meters>> {
    let from = mesh.nearest(from)?;
    mesh.nearest_where(goal, SPACING, |point| mesh.connects(from, point))
}

fn pointer_binding(action: InputAction) -> (MouseButton, u8, KeyModifiers) {
    bindings(action)
        .iter()
        .find_map(|binding| match *binding {
            InputBinding::Mouse { button, modifiers } => Some((button, 1, modifiers)),
            InputBinding::Click(gesture) => {
                Some((gesture.button, gesture.times, gesture.modifiers))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no click is bound to {action:?}"))
}

fn button_of(button: MouseButton) -> Button {
    match button {
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
        _ => Button::Left,
    }
}

fn modifier_keys(held: KeyModifiers) -> Vec<&'static str> {
    [
        (held.ctrl, "Control_L"),
        (held.shift, "Shift_L"),
        (held.alt, "Alt_L"),
        (held.super_key, "Super_L"),
    ]
    .into_iter()
    .filter_map(|(down, key)| down.then_some(key))
    .collect()
}

// xdotool names keys by their X keysyms.
fn keysym(key: KeyCode) -> String {
    let name = format!("{key:?}");
    match key {
        KeyCode::Space => "space".to_owned(),
        KeyCode::Enter => "Return".to_owned(),
        KeyCode::Backspace => "BackSpace".to_owned(),
        _ if name.starts_with("Key") => name["Key".len()..].to_lowercase(),
        _ if name.starts_with("Digit") => name["Digit".len()..].to_owned(),
        _ if name.starts_with("Arrow") => name["Arrow".len()..].to_owned(),
        _ => name,
    }
}

fn carry_binding() -> (MouseButton, KeyModifiers) {
    bindings(InputAction::CarryItem)
        .iter()
        .find_map(|binding| match *binding {
            InputBinding::Drag(gesture) => Some((gesture.button, gesture.modifiers)),
            _ => None,
        })
        .expect("a drag is bound to carrying items")
}
