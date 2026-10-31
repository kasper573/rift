//! Questions about the game itself are answered at once from the client's content rather than on
//! a frame, as a driver planning a walk asks hundreds of them in a row.

use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::MouseButton;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::content::ContentId;
use crate::core::content::{Content, ContentScope, Fixture, with_content};
use crate::core::geometry::Shape;
use crate::core::math::{Pos, PosSchema, Rect, RectSchema, Size, SizeSchema};
use crate::core::units::Meters;
use crate::systems::GameSettings;
use crate::systems::actor::ActorModel;
use crate::systems::actor::skeleton::BuildName;
use crate::systems::actor::stance::StanceId;
use crate::systems::area::{self, AreaDef, AreaId, AreaModule, PropId};
use crate::systems::input::map::{InputAction, InputBinding, InputMap, KeyModifiers};
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::{ItemDef, ItemId, Wear};
use crate::systems::movement::SPACING;
use crate::systems::npc::{NpcDef, NpcId};
use crate::systems::player::snapshot::CharacterSnapshot;
use crate::systems::script::Place;
use crate::systems::skill::{SkillDef, SkillId};

/// A question for the probe, one JSON line each, answered with one JSON line before the next is
/// asked. Positions are metres on the ground of the area asked about.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[serde(tag = "ask", rename_all = "snake_case")]
pub enum ProbeQuestion {
    /// Answered with a [`super::ProbeSnapshot`] of a frame after the question.
    Snapshot,
    /// Answered with [`ProbeGame`].
    Game,
    /// Each question about the game in turn, in one line rather than one each. Answered with a
    /// list of their answers, in the same order.
    Batch { questions: Vec<ProbeQuestion> },
    /// Whether `at` is walkable ground. Answered with a boolean.
    Walkable {
        area: AreaId,
        #[schemars(with = "PosSchema")]
        at: Pos<Meters>,
    },
    /// Whether a walk from `from` reaches `to`. Answered with a boolean.
    Connects {
        area: AreaId,
        #[schemars(with = "PosSchema")]
        from: Pos<Meters>,
        #[schemars(with = "PosSchema")]
        to: Pos<Meters>,
    },
    /// The corners of the walk from `from` to `to`, or null where none reaches. Answered with a
    /// list of positions.
    Route {
        area: AreaId,
        #[schemars(with = "PosSchema")]
        from: Pos<Meters>,
        #[schemars(with = "PosSchema")]
        to: Pos<Meters>,
    },
    /// The walkable ground nearest `at`, or null. Answered with a position.
    Nearest {
        area: AreaId,
        #[schemars(with = "PosSchema")]
        at: Pos<Meters>,
    },
    /// The walkable ground nearest `goal` that a walk from `from` reaches, searched along rings
    /// `spacing` apart, or null. Answered with a position.
    ReachableNear {
        area: AreaId,
        #[schemars(with = "PosSchema")]
        from: Pos<Meters>,
        #[schemars(with = "PosSchema")]
        goal: Pos<Meters>,
        spacing: Meters,
    },
    /// Whether `at` lies in a safe zone. Answered with a boolean.
    Safe {
        area: AreaId,
        #[schemars(with = "PosSchema")]
        at: Pos<Meters>,
    },
    /// Whether the terrain at `at` lays a picture of its own over the ground. Answered with a
    /// boolean.
    Pictured {
        area: AreaId,
        #[schemars(with = "PosSchema")]
        at: Pos<Meters>,
    },
}

/// The game's things: where each is, area by area, and what a player comes to know of them.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeGame {
    /// Where the game starts a new player.
    pub initial_spawn: Place,
    /// The characters a game server that hands out test characters plays.
    pub test_characters: Vec<ContentId<CharacterSnapshot>>,
    /// How far apart bodies keep on the ground.
    pub spacing: Meters,
    pub areas: Vec<ProbeArea>,
    /// The names of the keys and buttons bound to each input action, the first the one a player
    /// is told of.
    pub bindings: Vec<ProbeBindings>,
    pub npcs: Vec<ProbeNpc>,
    pub items: Vec<ProbeItem>,
    pub skills: Vec<ProbeSkill>,
    /// The images the interface shows its own icons with, by the icons' names.
    pub interface_icons: Vec<ProbeInterfaceIcon>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeInterfaceIcon {
    pub name: String,
    pub image: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeNpc {
    pub npc: NpcId,
    pub name: String,
    /// What it's listed as holding, whether or not its body has the hands to show it.
    pub held: Vec<ItemId>,
    /// The build its model is cut for, if its skeleton has builds.
    pub build: Option<BuildName>,
    /// Whether its build is given clips of its own, over those of the skeleton's first build.
    pub build_has_clips: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeItem {
    pub item: ItemId,
    pub name: String,
    /// The image the interface shows it by.
    pub icon: String,
    /// The stance holding it in a hand puts its holder in, for an item held rather than worn.
    pub stance: Option<StanceId>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeSkill {
    pub skill: SkillId,
    pub name: String,
    pub range: Option<Meters>,
    /// The ground it strikes around where it lands.
    pub area: Option<Shape>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeArea {
    pub area: AreaId,
    pub name: String,
    #[schemars(with = "SizeSchema")]
    pub size: Size<Meters>,
    /// The townsfolk who live here, each where they stand at home.
    pub residents: Vec<ProbeResident>,
    /// The monsters that roam here, each over its own corner of the area or all of it.
    pub roamings: Vec<ProbeRoaming>,
    /// The objects that carry a name, each at its middle.
    pub objects: Vec<ProbeNamedObject>,
    pub props: Vec<ProbeProp>,
    #[schemars(with = "Vec<PosSchema>")]
    pub lights: Vec<Pos<Meters>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeResident {
    pub npc: NpcId,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeRoaming {
    pub npc: NpcId,
    pub count: u32,
    #[schemars(with = "RectSchema")]
    pub over: Rect<Meters>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeNamedObject {
    pub name: String,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeProp {
    pub prop: PropId,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    /// Whether it fades out of the way of the camera's view of the player.
    pub fades: bool,
    /// Whether light shines through any of its materials.
    pub translucent: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeBindings {
    pub action: InputAction,
    pub bindings: Vec<ProbeBinding>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeBinding {
    /// What the player is told of it.
    pub name: String,
    pub gesture: ProbeGesture,
}

/// How a binding is made, with `held` the modifier keys held down while making it. Keys and
/// buttons go by the names bevy's `KeyCode` and `MouseButton` give them: `KeyI`, `Digit1`,
/// `Space`; `Left`, `Right`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[serde(tag = "gesture", rename_all = "snake_case")]
pub enum ProbeGesture {
    Key {
        #[schemars(with = "String")]
        key: KeyCode,
        held: KeyModifiers,
    },
    AnyKey,
    /// Pressed down, without waiting for its release.
    Press {
        #[schemars(with = "String")]
        button: MouseButton,
        held: KeyModifiers,
    },
    Click {
        #[schemars(with = "String")]
        button: MouseButton,
        times: u8,
        held: KeyModifiers,
    },
    Drag {
        #[schemars(with = "String")]
        button: MouseButton,
        held: KeyModifiers,
    },
}

impl From<InputBinding> for ProbeGesture {
    fn from(binding: InputBinding) -> ProbeGesture {
        match binding {
            InputBinding::Key(gesture) => ProbeGesture::Key {
                key: gesture.key,
                held: gesture.modifiers,
            },
            InputBinding::AnyKey => ProbeGesture::AnyKey,
            InputBinding::Mouse { button, modifiers } => ProbeGesture::Press {
                button,
                held: modifiers,
            },
            InputBinding::Click(gesture) => ProbeGesture::Click {
                button: gesture.button,
                times: gesture.times,
                held: gesture.modifiers,
            },
            InputBinding::Drag(gesture) => ProbeGesture::Drag {
                button: gesture.button,
                held: gesture.modifiers,
            },
        }
    }
}

/// What the game itself answers to `question`, or nothing for a question about a frame.
pub fn answer(content: &Content, question: &ProbeQuestion) -> Option<String> {
    let (answer, _) = with_content(ContentScope::of(content), || {
        answer_in_scope(content, question)
    });
    answer.map(|answer| answer.to_string())
}

fn answer_in_scope(content: &Content, question: &ProbeQuestion) -> Option<Value> {
    let answer = match question {
        ProbeQuestion::Snapshot => return None,
        ProbeQuestion::Game => json(&game(content)),
        ProbeQuestion::Batch { questions } => Value::Array(
            questions
                .iter()
                .map(|question| answer_in_scope(content, question))
                .collect::<Option<_>>()?,
        ),
        &ProbeQuestion::Walkable { area, at } => json(&mesh(content, area).walkable(at)),
        &ProbeQuestion::Connects { area, from, to } => {
            json(&mesh(content, area).connects(from, to))
        }
        &ProbeQuestion::Route { area, from, to } => json(&mesh(content, area).route(from, to)),
        &ProbeQuestion::Nearest { area, at } => json(&mesh(content, area).nearest(at)),
        &ProbeQuestion::ReachableNear {
            area,
            from,
            goal,
            spacing,
        } => {
            let mesh = mesh(content, area);
            let reached = mesh.nearest(from).and_then(|from| {
                mesh.nearest_where(goal, spacing, |point| mesh.connects(from, point))
            });
            json(&reached)
        }
        &ProbeQuestion::Safe { area, at } => json(&area::load(content, area).safe(at)),
        &ProbeQuestion::Pictured { area, at } => json(&pictured(content, area, at)),
    };
    Some(answer)
}

fn json(value: &impl Serialize) -> Value {
    serde_json::to_value(value).expect("a serializable answer")
}

fn game(content: &Content) -> ProbeGame {
    let inputs = InputMap::new(content);
    ProbeGame {
        initial_spawn: content.built::<GameSettings>().initial_spawn,
        test_characters: content.ids::<CharacterSnapshot>().collect(),
        spacing: SPACING,
        areas: content
            .ids::<AreaDef>()
            .map(|area| area_of(content, area))
            .collect(),
        bindings: <InputAction as strum::VariantArray>::VARIANTS
            .iter()
            .map(|&action| ProbeBindings {
                action,
                bindings: inputs
                    .bindings(action)
                    .iter()
                    .map(|binding| ProbeBinding {
                        name: binding.name(),
                        gesture: (*binding).into(),
                    })
                    .collect(),
            })
            .collect(),
        npcs: content
            .ids::<NpcDef>()
            .map(|npc| npc_of(content, npc))
            .collect(),
        items: content
            .ids::<ItemDef>()
            .map(|item| ProbeItem {
                item,
                name: item.get(content).name.clone(),
                icon: item.get(content).icon.path(),
                stance: match item.get(content).wear() {
                    Some(Wear::Held { stance, .. }) => Some(*stance),
                    _ => None,
                },
            })
            .collect(),
        skills: content
            .ids::<SkillDef>()
            .map(|skill| {
                let def = skill.get(content);
                ProbeSkill {
                    skill,
                    name: def.name.clone(),
                    range: def.range,
                    area: def.area,
                }
            })
            .collect(),
        interface_icons: <InterfaceIcon as strum::VariantArray>::VARIANTS
            .iter()
            .map(|&icon| ProbeInterfaceIcon {
                name: icon.name().to_owned(),
                image: icon.get(content).image.path(),
            })
            .collect(),
    }
}

fn npc_of(content: &Content, npc: NpcId) -> ProbeNpc {
    let def = npc.get(content);
    let model = ActorModel::get(content, def.appearance.model);
    ProbeNpc {
        npc,
        name: def.name.clone(),
        held: def.held.clone(),
        build: model.build().cloned(),
        build_has_clips: model.build_has_clips(),
    }
}

fn area_of(content: &Content, id: AreaId) -> ProbeArea {
    let area = area::load(content, id);
    let residents = area::modules(content, id)
        .filter_map(|module| match module {
            AreaModule::Resident(resident) => Some(ProbeResident {
                npc: resident.npc,
                at: resident.at,
            }),
            _ => None,
        })
        .collect();
    let roamings = area::modules(content, id)
        .filter_map(|module| match module {
            AreaModule::Population(population) => Some(ProbeRoaming {
                npc: population.npc,
                count: population.count,
                over: population
                    .roams
                    .unwrap_or_else(|| Rect::from_size(area.size)),
            }),
            _ => None,
        })
        .collect();
    let objects = area::objects(area::placed(content, id))
        .filter_map(|(_, object)| {
            Some(ProbeNamedObject {
                name: object.name.clone()?,
                at: object.shape.center(),
            })
        })
        .collect();
    let props = area
        .props
        .iter()
        .map(|placement| {
            let prop = placement.prop.get(content);
            ProbeProp {
                prop: placement.prop,
                at: placement.at,
                fades: prop.fade,
                translucent: prop
                    .materials
                    .values()
                    .any(|material| material.get(content).translucency > 0.0),
            }
        })
        .collect();
    ProbeArea {
        area: id,
        name: id.get(content).name.to_string(),
        size: area.size,
        residents,
        roamings,
        objects,
        props,
        lights: area.lights.iter().map(|light| light.at).collect(),
    }
}

fn mesh(content: &Content, area: AreaId) -> &crate::systems::movement::NavMesh {
    area::load(content, area).mesh()
}

fn pictured(content: &Content, id: AreaId, at: Pos<Meters>) -> bool {
    let area = area::load(content, id);
    area.terrains
        .get(usize::from(area.paint.at(at)))
        .is_some_and(|terrain| terrain.get(content).picture.is_some())
}
