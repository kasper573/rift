//! Bodies stood off screen as the game stands them: a model dressed as its look says, posed in a
//! clip of its skeleton or in its stance's idle, and holding its items in its hands.

use std::collections::BTreeMap;

use bevy::prelude::*;
use engine::core::content::Content;
use engine::core::render::world::body;
use engine::core::render::world::layers::Layered;
use engine::core::render::world::mannequin::mannequin;
use engine::core::render::world::plumb::Plumb;
use engine::core::render::world::rig_clip;
use engine::core::render::world::shine::HeldPiece;
use engine::core::units::Seconds;
use engine::systems::actor::layering::{self, HandHold, Layering};
use engine::systems::actor::model::{ActorModel, ModelId};
use engine::systems::actor::skeleton::Clip;
use engine::systems::actor::stance::{self, HandIndex, StanceId};
use engine::systems::actor::{Action, Appearance, ModelDef, Motion, Pose, appearance, render};
use engine::systems::area::PropId;
use engine::systems::area::scenery::ShowsProp;
use engine::systems::item::{ItemDef, PropHold, Steady, Wear};

/// How a body stands: its model by its path, the items in its hands by theirs, hand by hand, and
/// the clip it plays, its stance's idle unless given, held at `moment` or played on a loop.
pub struct Posed<'a> {
    pub model: &'a str,
    pub held: &'a [Option<String>],
    pub clip: Option<&'a Clip>,
    pub moment: Option<Seconds>,
}

/// A body stood: its rig, and the props that hang from its hands once its joints stand.
pub struct Body {
    pub rig: Entity,
    model: ModelId,
    stance: Option<StanceId>,
    props: Vec<Prop>,
    hung: bool,
}

struct Prop {
    hand: HandIndex,
    joint: String,
    look: PropId,
    transform: Transform,
    plumb: Option<Plumb>,
    hold: PropHold,
    steady: Option<Steady>,
}

/// Stands a body of `posed` at `placed`.
pub fn stand(world: &mut World, content: &Content, posed: &Posed, placed: Transform) -> Body {
    let model_id = content
        .by_path::<ModelDef>(posed.model)
        .unwrap_or_else(|| panic!("no model {}", posed.model));
    let model = loaded(content, model_id);
    let held: BTreeMap<HandIndex, _> = posed
        .held
        .iter()
        .enumerate()
        .filter_map(|(hand, item)| {
            let item = item.as_ref()?;
            let id = content
                .by_path::<ItemDef>(item)
                .unwrap_or_else(|| panic!("no item {item}"));
            Some((HandIndex(hand as u8), id))
        })
        .collect();
    let stance = stance::of_hands(content, &held);
    let built = model.skeleton();
    let played = match posed.clip {
        Some(clip) => built
            .played(clip)
            .unwrap_or_else(|| panic!("{} plays no clip {clip}", posed.model)),
        None => model.standing_clip(stance, &Motion::Action(Action::Idle), Pose::Idle),
    };
    let clip = &built.clips[played];
    let pairs = clip
        .source
        .map(|source| built.def.sources[source].joint_pairs());
    let made = rig_clip::rig_clip(
        world,
        built.rig.clone(),
        clip.clip.file().as_str(),
        clip.animation,
        rig_clip::maker(built.rig.clone(), pairs),
    );
    let mut mannequin = mannequin(world, made);
    mannequin.hold(posed.moment);
    let figure = model.dressed(
        content,
        &appearance::look(content, &Appearance::of(model_id), &BTreeMap::new()),
    );
    let rig = world.spawn((placed, Visibility::default(), mannequin)).id();
    body::dress(world, rig, &figure);
    let hung = render::hung(content, model, &held);
    for item in held.values() {
        assert!(
            hung.values().any(|hanging| hanging == item),
            "{} holds {} in no hand: list it for every hand its stance holds it with",
            posed.model,
            item.path(content)
        );
    }
    let props = hung
        .into_iter()
        .filter_map(|(hand, item)| {
            let (_, grip) = item.get(content).wear().and_then(Wear::grasp)?;
            let at = model.hands().get(hand.index())?;
            Some(Prop {
                hand,
                joint: at.joint.0.clone(),
                look: item.get(content).look,
                transform: render::held_at(model, at, grip),
                plumb: render::plumb_from(model, at, grip),
                hold: grip.hold,
                steady: grip.steady,
            })
        })
        .collect();
    Body {
        rig,
        model: model_id,
        stance,
        props,
        hung: false,
    }
}

impl Body {
    pub fn model<'c>(&self, content: &'c Content) -> &'c ActorModel {
        loaded(content, self.model)
    }

    /// Hangs the body's props under their joints and puts its layers on, once its scene has
    /// spawned the joints and every part of it shows: whether it has.
    pub fn hang(&mut self, world: &mut World, content: &Content) -> bool {
        if self.hung {
            return true;
        }
        let model = loaded(content, self.model);
        let Some(joints) = world
            .get::<body::Body>(self.rig)
            .map(|body| body.joints().clone())
        else {
            return false;
        };
        let rigged = model
            .skeleton()
            .rig
            .joints()
            .all(|joint| joints.contains_key(&model.skeleton().rig.nodes[joint].name));
        let dressed = world
            .get::<Children>(self.rig)
            .into_iter()
            .flatten()
            .filter(|child| world.get::<WorldAssetRoot>(**child).is_some())
            .all(|skin| world.get::<Visibility>(*skin) != Some(&Visibility::Hidden));
        if !rigged || !dressed {
            return false;
        }
        for prop in &self.props {
            let Some(&joint) = joints.get(&prop.joint) else {
                continue;
            };
            let mut hung = world.spawn((
                HeldPiece,
                ShowsProp(prop.look),
                prop.transform,
                Visibility::default(),
                ChildOf(joint),
            ));
            if let Some(plumb) = prop.plumb {
                hung.insert(plumb);
            }
        }
        if let Some((layered, layering)) = layering::layered(model, &joints) {
            world.entity_mut(self.rig).insert((layered, layering));
        }
        self.hung = true;
        true
    }
}

/// Drives each body's layers as the game drives an idle body holding its props.
pub fn drive<'b>(world: &mut World, bodies: impl IntoIterator<Item = &'b Body>) {
    for standing in bodies {
        let holding: Vec<HandHold> = standing
            .props
            .iter()
            .map(|prop| (prop.hand, prop.hold, prop.steady))
            .collect();
        if let Ok((mut layered, layering)) = world
            .query::<(&mut Layered, &Layering)>()
            .get_mut(world, standing.rig)
        {
            layering::drive(
                &mut layered,
                layering,
                Action::Idle,
                Pose::Idle,
                standing.stance,
                &holding,
                false,
            );
        }
    }
}

fn loaded(content: &Content, model: ModelId) -> &ActorModel {
    ActorModel::loaded(content, model)
        .unwrap_or_else(|| panic!("{} did not build", model.path(content)))
}
