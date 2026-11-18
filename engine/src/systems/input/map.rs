use std::collections::{BTreeMap, HashMap};
use ui::egui;

use bevy::input::InputSystems;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy_egui::helpers::bevy_to_egui_physical_key;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use strum::{EnumDiscriminants, IntoStaticStr, VariantArray};

use super::InputMessages;
use super::scope;
use crate::core::content::SchemaMark;
use crate::core::content::fixture::FixtureLock;
use crate::core::content::{Content, ContentFile, Fixture, Nearby, RowCheck, serial};
use crate::core::text::{
    InputRef, KeyNames, PluggedSpan, SpanDef, SpanKinds, SpanText, TextKey, TextPiece,
};
use crate::systems::messages;
use ui::keys::{CapFace, KeyCap, MouseGlyph};

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum InputAction {
    Interact,
    Select,
    InspectItem,
    UseItem,
    DropItem,
    /// Asks how many to split off a stack of items into a stack of its own.
    SplitStack,
    PlaceItem,
    PlaceStack,
    CarryItem,
    OpenCharacter,
    OpenSkills,
    OpenQuests,
    OpenMap,
    OpenHistory,
    OpenSocial,
    OpenSettings,
    /// Accepts the request on the card in front.
    AcceptRequest,
    /// Denies the request on the card in front.
    DenyRequest,
    Dismiss,
    SubmitText,
    /// Takes the selected way to go on with what is being typed.
    CompleteText,
    /// Turns what is being read a page back, toward what came earlier.
    PageBack,
    PageForward,
    /// Shows the next of chat's tabs, round to the first from the last.
    NextChatTab,
    PreviousChatTab,
    TakeDefault,
    Respawn,
    TurnLeft,
    TurnRight,
    TiltUp,
    TiltDown,
    CameraHome,
    TurnCamera,
    TiltCamera,
    /// Brings the camera closer, or a map the pointer is on.
    ZoomIn,
    ZoomOut,
    ChoosePrevious,
    ChooseNext,
    PickChoice,
    Advance,
    Choice1,
    Choice2,
    Choice3,
    Choice4,
    Choice5,
    Choice6,
    Choice7,
    Choice8,
    Choice9,
    BarSlot1,
    BarSlot2,
    BarSlot3,
    BarSlot4,
    BarSlot5,
    BarSlot6,
    BarSlot7,
    BarSlot8,
    BarSlot9,
    PinToBar,
    RaiseRank,
    LowerRank,
    InspectSkill,
    UseSlot,
    SlotOptions,
    AddToChat,
    /// Held, to keep the wheel of emotes open round the player's character.
    Emotes,
    /// Held or tapped, to name the items lying round the player's character.
    NameItems,
    FocusChat,
    /// Leaves the seat of what the player's character rides.
    StepOff,
    /// Opens what the player can do with the other player the pointer is on.
    ActorOptions,
}

/// The one file that binds every action, a fixture every game provides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum InputFixture {
    Game,
}

impl Fixture for InputFixture {
    type Row = InputDef;
    const FOLDER: Option<&'static str> = None;
    const WHOLE_TABLE: bool = true;

    fn locks(self) -> Vec<FixtureLock> {
        InputAction::VARIANTS
            .iter()
            .map(|&action| FixtureLock::Variants {
                list: format!("{}/bindings", action.pointer()),
                variants: action
                    .readings()
                    .iter()
                    .flat_map(|gesture| gesture.bindings())
                    .map(|kind| <&'static str>::from(*kind).to_owned())
                    .collect(),
            })
            .collect()
    }

    fn fresh(self, _: &Nearby) -> Result<InputDef, String> {
        Ok(InputDef {
            actions: InputAction::VARIANTS
                .iter()
                .map(|&action| {
                    let label = <&'static str>::from(action).replace('_', " ");
                    let bound = BoundAction {
                        label,
                        bindings: Vec::new(),
                    };
                    (action, bound)
                })
                .collect(),
        })
    }
}

impl InputAction {
    /// Where the action sits in the input file.
    pub fn pointer(self) -> String {
        format!("/actions/{}", <&'static str>::from(self))
    }

    pub fn readings(self) -> &'static [ActionReading] {
        match self {
            InputAction::Advance | InputAction::PickChoice => {
                &[ActionReading::Press, ActionReading::Click]
            }
            InputAction::Select
            | InputAction::InspectItem
            | InputAction::UseItem
            | InputAction::DropItem
            | InputAction::SplitStack
            | InputAction::PlaceItem
            | InputAction::PlaceStack
            | InputAction::PinToBar
            | InputAction::RaiseRank
            | InputAction::LowerRank
            | InputAction::InspectSkill
            | InputAction::UseSlot
            | InputAction::SlotOptions
            | InputAction::AddToChat => &[ActionReading::Click],
            InputAction::CarryItem => &[ActionReading::Drag],
            InputAction::SubmitText
            | InputAction::CompleteText
            | InputAction::PageBack
            | InputAction::PageForward
            | InputAction::NextChatTab
            | InputAction::PreviousChatTab
            | InputAction::TakeDefault => &[ActionReading::Shortcut],
            InputAction::ZoomIn | InputAction::ZoomOut => {
                &[ActionReading::Press, ActionReading::Wheel]
            }
            _ => &[ActionReading::Press],
        }
    }
}

/// The ways the game reads an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionReading {
    Press,
    Click,
    Drag,
    Wheel,
    Shortcut,
}

impl ActionReading {
    /// The bindings the game reads for the gesture.
    pub fn bindings(self) -> &'static [InputBindingKind] {
        match self {
            ActionReading::Press => &[
                InputBindingKind::Key,
                InputBindingKind::AnyKey,
                InputBindingKind::Mouse,
            ],
            ActionReading::Click => &[InputBindingKind::Click],
            ActionReading::Drag => &[InputBindingKind::Drag],
            ActionReading::Wheel => &[InputBindingKind::Wheel],
            ActionReading::Shortcut => &[InputBindingKind::Key],
        }
    }
}

/// What every action is called and bound to. The engine fixes which actions there are; content
/// words them for the player and picks their bindings, of the kinds each action reads.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDef {
    #[schemars(schema_with = "actions_schema")]
    pub actions: BTreeMap<InputAction, BoundAction>,
}

/// An action as the player reads it and binds it.
#[derive(JsonSchema, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundAction {
    pub label: String,
    pub bindings: Vec<InputBinding>,
}

impl ContentFile for InputDef {
    const TABLE: &'static str = "input";

    fn fresh(near: &Nearby) -> Result<InputDef, String> {
        InputFixture::Game.fresh(near)
    }

    fn check(&self, cx: &mut RowCheck) {
        for &action in InputAction::VARIANTS {
            match self.actions.get(&action) {
                Some(bound) => cx.text(format!("{}/label", action.pointer()), &bound.label, 1, 80),
                None => cx.error(
                    action.pointer(),
                    "missing input",
                    "no such action",
                    "every action the engine reads",
                ),
            }
        }
    }
}

/// Every action keys a row of its own: a sheet lists them, and none is added, removed or renamed.
fn actions_schema(generator: &mut SchemaGenerator) -> Schema {
    let actions = generator.subschema_for::<InputAction>();
    let bound = generator.subschema_for::<BoundAction>();
    let mut schema = json_schema!({
        "type": "object",
        "propertyNames": actions,
        "additionalProperties": bound,
    });
    SchemaMark::FixedRows.set(&mut schema, true);
    schema
}

#[derive(
    JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, EnumDiscriminants,
)]
#[serde(tag = "type", rename_all = "snake_case")]
#[strum_discriminants(
    name(InputBindingKind),
    derive(IntoStaticStr),
    strum(serialize_all = "snake_case")
)]
pub enum InputBinding {
    Key(KeyGesture),
    AnyKey,
    Mouse {
        #[schemars(schema_with = "mouse_button_schema")]
        button: MouseButton,
        #[serde(flatten)]
        modifiers: KeyModifiers,
    },
    Click(ClickGesture),
    Drag(DragGesture),
    Wheel(WheelGesture),
}

impl InputBinding {
    /// Whether this binding answers wherever `other` would, before `other` can: the same
    /// gesture, or a press or a shorter run of clicks of the same button with the same modifiers,
    /// which fires on the first click of `other`'s run.
    pub fn preempts(&self, other: &InputBinding) -> bool {
        match (self, other) {
            (InputBinding::Click(first), InputBinding::Click(run)) => {
                first.button == run.button
                    && first.modifiers == run.modifiers
                    && first.times <= run.times
            }
            (InputBinding::Mouse { button, modifiers }, InputBinding::Click(run)) => {
                *button == run.button && *modifiers == run.modifiers
            }
            _ => self == other,
        }
    }

    pub const fn key(key: KeyCode) -> InputBinding {
        InputBinding::Key(KeyGesture {
            key,
            modifiers: KeyModifiers::NONE,
        })
    }

    pub const fn mouse(button: MouseButton) -> InputBinding {
        InputBinding::Mouse {
            button,
            modifiers: KeyModifiers::NONE,
        }
    }

    pub const fn click(button: MouseButton) -> InputBinding {
        InputBinding::Click(ClickGesture {
            button,
            times: Clicks::Single,
            modifiers: KeyModifiers::NONE,
        })
    }

    pub const fn drag(button: MouseButton) -> InputBinding {
        InputBinding::Drag(DragGesture {
            button,
            modifiers: KeyModifiers::NONE,
        })
    }

    pub const fn wheel(turn: WheelTurn) -> InputBinding {
        InputBinding::Wheel(WheelGesture {
            turn,
            modifiers: KeyModifiers::NONE,
        })
    }

    pub const fn with(self, held: KeyModifiers) -> InputBinding {
        match self {
            InputBinding::Key(gesture) => InputBinding::Key(KeyGesture {
                modifiers: gesture.modifiers.and(held),
                ..gesture
            }),
            InputBinding::AnyKey => InputBinding::AnyKey,
            InputBinding::Mouse { button, modifiers } => InputBinding::Mouse {
                button,
                modifiers: modifiers.and(held),
            },
            InputBinding::Click(gesture) => InputBinding::Click(ClickGesture {
                modifiers: gesture.modifiers.and(held),
                ..gesture
            }),
            InputBinding::Drag(gesture) => InputBinding::Drag(DragGesture {
                modifiers: gesture.modifiers.and(held),
                ..gesture
            }),
            InputBinding::Wheel(gesture) => InputBinding::Wheel(WheelGesture {
                modifiers: gesture.modifiers.and(held),
                ..gesture
            }),
        }
    }

    fn modifiers(self) -> KeyModifiers {
        match self {
            InputBinding::Key(gesture) => gesture.modifiers,
            InputBinding::AnyKey => KeyModifiers::NONE,
            InputBinding::Mouse { modifiers, .. } => modifiers,
            InputBinding::Click(gesture) => gesture.modifiers,
            InputBinding::Drag(gesture) => gesture.modifiers,
            InputBinding::Wheel(gesture) => gesture.modifiers,
        }
    }

    /// The gesture alone, with nothing held beside it.
    fn bare(self) -> InputBinding {
        match self {
            InputBinding::Key(gesture) => InputBinding::key(gesture.key),
            InputBinding::AnyKey => InputBinding::AnyKey,
            InputBinding::Mouse { button, .. } => InputBinding::mouse(button),
            InputBinding::Click(gesture) => InputBinding::Click(ClickGesture {
                modifiers: KeyModifiers::NONE,
                ..gesture
            }),
            InputBinding::Drag(gesture) => InputBinding::drag(gesture.button),
            InputBinding::Wheel(gesture) => InputBinding::wheel(gesture.turn),
        }
    }

    /// What a player reads for the binding: its name, and the key cap or mouse glyph it is drawn as.
    pub fn cap(&self, words: &InputMessages) -> KeyCap {
        let modifiers = self.modifiers();
        let (face, name) = match *self {
            InputBinding::Key(gesture) => {
                let name = key_name(words, gesture.key);
                (CapFace::Text(name.clone()), name)
            }
            InputBinding::AnyKey => (
                CapFace::Text(words.any_key().to_owned()),
                words.any_key().to_owned(),
            ),
            InputBinding::Mouse { button, .. } => mouse_cap(words, button, Clicks::Single, false),
            InputBinding::Click(gesture) => mouse_cap(words, gesture.button, gesture.times, false),
            InputBinding::Drag(gesture) => mouse_cap(words, gesture.button, Clicks::Single, true),
            InputBinding::Wheel(gesture) => {
                let (glyph, name) = match gesture.turn {
                    WheelTurn::Up => (MouseGlyph::WheelUp, words.wheel_up()),
                    WheelTurn::Down => (MouseGlyph::WheelDown, words.wheel_down()),
                };
                let face = CapFace::Mouse {
                    glyph,
                    presses: 1,
                    drag: false,
                };
                (face, name.to_owned())
            }
        };
        let held = held_names(words, modifiers);
        let name = held
            .iter()
            .map(|held| format!("{held}+"))
            .chain([name])
            .collect();
        KeyCap { held, face, name }
    }

    pub fn name(&self, words: &InputMessages) -> String {
        self.cap(words).name
    }
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct InputMap {
    words: InputMessages,
    bindings: HashMap<InputAction, Vec<InputBinding>>,
    own_keys: KeyModifiers,
    /// Every modifier the game binds a gesture with, by the gesture alone.
    bound_with: HashMap<InputBinding, KeyModifiers>,
}

impl InputMap {
    pub fn new(content: &Content) -> InputMap {
        let file = InputFixture::Game.get(content);
        let bindings: HashMap<InputAction, Vec<InputBinding>> = InputAction::VARIANTS
            .iter()
            .map(|&action| {
                let bound = file.actions.get(&action).cloned().unwrap_or_default();
                (action, bound.bindings)
            })
            .collect();
        let own_keys = bindings
            .values()
            .flatten()
            .filter_map(|binding| match binding {
                InputBinding::Key(gesture) => Some(KeyModifiers::of(gesture.key)),
                _ => None,
            })
            .fold(KeyModifiers::NONE, KeyModifiers::and);
        let mut bound_with: HashMap<InputBinding, KeyModifiers> = HashMap::new();
        for binding in bindings.values().flatten() {
            let with = bound_with.entry(binding.bare()).or_default();
            *with = with.and(binding.modifiers());
        }
        InputMap {
            words: messages::of(content).input.clone(),
            bindings,
            own_keys,
            bound_with,
        }
    }

    pub fn words(&self) -> &InputMessages {
        &self.words
    }

    pub fn bindings(&self, action: InputAction) -> &[InputBinding] {
        self.bindings.get(&action).map_or(&[], Vec::as_slice)
    }

    /// Whether a binding of one of `first`, read before `action` where both are, answers before a
    /// binding of `action` can.
    pub fn preempted(&self, first: &[InputAction], action: InputAction) -> bool {
        first.iter().any(|&before| {
            self.bindings(before).iter().any(|binding| {
                self.bindings(action)
                    .iter()
                    .any(|other| binding.preempts(other))
            })
        })
    }

    /// Whether the modifiers held are just those `gesture` asks for. A modifier the game binds
    /// as an action's own key is that key: held, it stands in no gesture's way, except where the
    /// game also binds the gesture with it, and then it means that binding.
    fn holds(&self, keys: &ButtonInput<KeyCode>, gesture: InputBinding) -> bool {
        let wanted = gesture.modifiers();
        let meant = self
            .bound_with
            .get(&gesture.bare())
            .map_or(wanted, |with| with.and(wanted));
        KeyModifiers::held(keys).without(self.own_keys.without(meant)) == wanted
    }

    pub fn cap(&self, action: InputAction) -> Option<KeyCap> {
        self.bindings(action)
            .first()
            .map(|binding| binding.cap(&self.words))
    }

    pub fn shortcut(&self, action: InputAction) -> Option<egui::KeyboardShortcut> {
        self.bindings(action)
            .iter()
            .find_map(|binding| match binding {
                InputBinding::Key(gesture) => Some(egui::KeyboardShortcut::new(
                    gesture.modifiers.egui(),
                    bevy_to_egui_physical_key(&gesture.key)?,
                )),
                _ => None,
            })
    }

    /// The cap every action is bound to, as lines name their keys.
    pub fn key_names(&self) -> KeyNames {
        KeyNames(
            InputAction::VARIANTS
                .iter()
                .filter_map(|&action| Some((action.into(), self.cap(action)?)))
                .collect(),
        )
    }
}

/// Whether `action` was pressed this frame, in a place that reads it.
pub fn just_pressed(world: &World, action: InputAction) -> bool {
    let keys = world.resource::<ButtonInput<KeyCode>>();
    let mouse = world.resource::<ButtonInput<MouseButton>>();
    let map = world.resource::<InputMap>();
    scope::reads(world, action)
        && map.bindings(action).iter().any(|binding| match *binding {
            InputBinding::Key(gesture) => gesture.just_pressed(keys, map),
            InputBinding::AnyKey => keys.get_just_pressed().next().is_some(),
            InputBinding::Mouse { button, .. } => {
                mouse.just_pressed(button) && map.holds(keys, *binding)
            }
            InputBinding::Click(_) | InputBinding::Drag(_) | InputBinding::Wheel(_) => false,
        })
}

pub fn just_pressed_index(world: &World, actions: &[InputAction]) -> Option<usize> {
    actions
        .iter()
        .position(|action| just_pressed(world, *action))
}

pub fn pressed(world: &World, action: InputAction) -> bool {
    let keys = world.resource::<ButtonInput<KeyCode>>();
    let mouse = world.resource::<ButtonInput<MouseButton>>();
    let map = world.resource::<InputMap>();
    scope::reads(world, action)
        && map.bindings(action).iter().any(|binding| match *binding {
            InputBinding::Key(gesture) => gesture.pressed(keys, map),
            InputBinding::AnyKey => keys.get_pressed().next().is_some(),
            InputBinding::Mouse { button, .. } => {
                mouse.pressed(button) && map.holds(keys, *binding)
            }
            InputBinding::Click(_) | InputBinding::Drag(_) | InputBinding::Wheel(_) => false,
        })
}

pub fn clicked(world: &World, action: InputAction, response: &egui::Response) -> bool {
    let keys = world.resource::<ButtonInput<KeyCode>>();
    let map = world.resource::<InputMap>();
    map.bindings(action).iter().any(|binding| match binding {
        InputBinding::Click(gesture) => gesture.clicked(response, keys, map),
        _ => false,
    })
}

pub fn drag_started(world: &World, action: InputAction, response: &egui::Response) -> bool {
    let keys = world.resource::<ButtonInput<KeyCode>>();
    let map = world.resource::<InputMap>();
    map.bindings(action).iter().any(|binding| match binding {
        InputBinding::Drag(gesture) => gesture.started(response, keys, map),
        _ => false,
    })
}

/// How many notches the wheel turned for `action` this frame. Like a click, a turn belongs to
/// whatever the pointer is on, so whoever asks has it.
pub fn wheeled(world: &World, action: InputAction) -> u32 {
    let keys = world.resource::<ButtonInput<KeyCode>>();
    let turned = world
        .get_resource::<WheelNotches>()
        .map_or(0, |notches| notches.turned);
    let map = world.resource::<InputMap>();
    map.bindings(action)
        .iter()
        .filter_map(|binding| match binding {
            InputBinding::Wheel(gesture) => Some(gesture.notches(turned, keys, map)),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

/// Whether a turn of the wheel is one of the ways `action` is bound.
pub fn takes_wheel(world: &World, action: InputAction) -> bool {
    world
        .resource::<InputMap>()
        .bindings(action)
        .iter()
        .any(|binding| matches!(binding, InputBinding::Wheel(_)))
}

pub fn input(action: InputAction) -> TextPiece {
    TextPiece::Key(TextKey::Input(action.into()))
}

/// Registers the kinds of span this system draws.
pub fn spans(kinds: &mut SpanKinds) {
    kinds.add::<ActionCap>("action_cap");
    kinds.add::<BindingCap>("binding_cap");
}

/// A key or button as a cap in a line, showing how the action is bound now.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ActionCap(pub InputAction);

/// A key or button as a cap in a line, bound to no action of the game's own.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct BindingCap(pub InputBinding);

impl SpanDef for ActionCap {
    fn pieces(&self, _: &Content) -> Vec<TextPiece> {
        vec![input(self.0)]
    }

    fn words(&self, _: &Content, keys: Option<&KeyNames>) -> String {
        keys.and_then(|keys| keys.0.get(&InputRef::from(self.0)))
            .map(|cap| cap.name.clone())
            .unwrap_or_default()
    }

    fn word_count(&self, _: &Content) -> usize {
        1
    }
}

impl SpanDef for BindingCap {
    fn pieces(&self, content: &Content) -> Vec<TextPiece> {
        let cap = self.0.cap(&messages::of(content).input);
        vec![TextPiece::Key(TextKey::Cap(cap))]
    }

    fn words(&self, content: &Content, keys: Option<&KeyNames>) -> String {
        keys.map(|_| self.0.cap(&messages::of(content).input).name)
            .unwrap_or_default()
    }

    fn word_count(&self, _: &Content) -> usize {
        1
    }
}

impl From<InputAction> for SpanText {
    fn from(action: InputAction) -> SpanText {
        SpanText::Part(PluggedSpan::new(ActionCap(action)))
    }
}

impl From<InputBinding> for SpanText {
    fn from(binding: InputBinding) -> SpanText {
        SpanText::Part(PluggedSpan::new(BindingCap(binding)))
    }
}

/// A line of hints, each an input followed by what it does, with a dot between one and the next.
pub fn hints<S: AsRef<str>>(hints: impl IntoIterator<Item = (InputAction, S)>) -> Vec<TextPiece> {
    hints
        .into_iter()
        .enumerate()
        .flat_map(|(index, (action, does))| {
            let separator = (index > 0).then(|| TextPiece::text(" · "));
            separator.into_iter().chain([
                input(action),
                TextPiece::text(format!(" {}", does.as_ref())),
            ])
        })
        .collect()
}

impl From<InputAction> for InputRef {
    fn from(action: InputAction) -> InputRef {
        InputRef(action as u16)
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<WheelNotches>().add_systems(
        PreUpdate,
        (
            publish.run_if(resource_exists_and_changed::<InputMap>),
            count_notches.after(InputSystems),
        ),
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct KeyGesture {
    #[schemars(schema_with = "key_code_schema")]
    pub key: KeyCode,
    #[serde(flatten)]
    pub modifiers: KeyModifiers,
}

impl KeyGesture {
    pub fn just_pressed(&self, keys: &ButtonInput<KeyCode>, map: &InputMap) -> bool {
        keys.just_pressed(self.key) && map.holds(keys, InputBinding::Key(*self))
    }

    pub fn pressed(&self, keys: &ButtonInput<KeyCode>, map: &InputMap) -> bool {
        keys.pressed(self.key) && map.holds(keys, InputBinding::Key(*self))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct ClickGesture {
    #[schemars(schema_with = "mouse_button_schema")]
    pub button: MouseButton,
    #[schemars(with = "u8", extend("minimum" = 1, "maximum" = 3), transform = SchemaMark::Count.with(true))]
    pub times: Clicks,
    #[serde(flatten)]
    pub modifiers: KeyModifiers,
}

/// How many clicks in a row make a gesture, as many as a pointer tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum Clicks {
    Single,
    Double,
    Triple,
}

impl TryFrom<u8> for Clicks {
    type Error = String;

    fn try_from(times: u8) -> Result<Clicks, String> {
        match times {
            1 => Ok(Clicks::Single),
            2 => Ok(Clicks::Double),
            3 => Ok(Clicks::Triple),
            _ => Err(format!("{times} clicks, not 1 to 3")),
        }
    }
}

impl From<Clicks> for u8 {
    fn from(times: Clicks) -> u8 {
        match times {
            Clicks::Single => 1,
            Clicks::Double => 2,
            Clicks::Triple => 3,
        }
    }
}

impl ClickGesture {
    fn clicked(
        &self,
        response: &egui::Response,
        keys: &ButtonInput<KeyCode>,
        map: &InputMap,
    ) -> bool {
        let Some(button) = pointer_button(self.button) else {
            return false;
        };
        let clicked = match self.times {
            Clicks::Single => response.clicked_by(button),
            Clicks::Double => response.double_clicked_by(button),
            Clicks::Triple => response.triple_clicked_by(button),
        };
        clicked && map.holds(keys, InputBinding::Click(*self))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct DragGesture {
    #[schemars(schema_with = "mouse_button_schema")]
    pub button: MouseButton,
    #[serde(flatten)]
    pub modifiers: KeyModifiers,
}

impl DragGesture {
    fn started(
        &self,
        response: &egui::Response,
        keys: &ButtonInput<KeyCode>,
        map: &InputMap,
    ) -> bool {
        pointer_button(self.button).is_some_and(|button| response.drag_started_by(button))
            && map.holds(keys, InputBinding::Drag(*self))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct WheelGesture {
    pub turn: WheelTurn,
    #[serde(flatten)]
    pub modifiers: KeyModifiers,
}

/// Which way the wheel turns: up is away from the player's hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WheelTurn {
    Up,
    Down,
}

impl WheelGesture {
    fn notches(&self, turned: i32, keys: &ButtonInput<KeyCode>, map: &InputMap) -> u32 {
        if !map.holds(keys, InputBinding::Wheel(*self)) {
            return 0;
        }
        match self.turn {
            WheelTurn::Up => turned.max(0).unsigned_abs(),
            WheelTurn::Down => turned.min(0).unsigned_abs(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct KeyModifiers {
    #[serde(skip_serializing_if = "serial::is_false")]
    pub ctrl: bool,
    #[serde(skip_serializing_if = "serial::is_false")]
    pub shift: bool,
    #[serde(skip_serializing_if = "serial::is_false")]
    pub alt: bool,
    #[serde(skip_serializing_if = "serial::is_false")]
    pub super_key: bool,
}

impl KeyModifiers {
    pub const NONE: KeyModifiers = KeyModifiers {
        ctrl: false,
        shift: false,
        alt: false,
        super_key: false,
    };

    pub const fn and(self, other: KeyModifiers) -> KeyModifiers {
        KeyModifiers {
            ctrl: self.ctrl || other.ctrl,
            shift: self.shift || other.shift,
            alt: self.alt || other.alt,
            super_key: self.super_key || other.super_key,
        }
    }

    const fn without(self, other: KeyModifiers) -> KeyModifiers {
        KeyModifiers {
            ctrl: self.ctrl && !other.ctrl,
            shift: self.shift && !other.shift,
            alt: self.alt && !other.alt,
            super_key: self.super_key && !other.super_key,
        }
    }

    /// The modifier `key` is, when it is one.
    fn of(key: KeyCode) -> KeyModifiers {
        KeyModifiers {
            ctrl: CTRL.contains(&key),
            shift: SHIFT.contains(&key),
            alt: ALT.contains(&key),
            super_key: SUPER.contains(&key),
        }
    }

    pub fn held(keys: &ButtonInput<KeyCode>) -> KeyModifiers {
        KeyModifiers {
            ctrl: keys.any_pressed(CTRL),
            shift: keys.any_pressed(SHIFT),
            alt: keys.any_pressed(ALT),
            super_key: keys.any_pressed(SUPER),
        }
    }

    pub fn is_modifier(key: KeyCode) -> bool {
        KeyModifiers::of(key) != KeyModifiers::NONE
    }

    fn egui(self) -> egui::Modifiers {
        egui::Modifiers {
            alt: self.alt,
            ctrl: self.ctrl,
            shift: self.shift,
            mac_cmd: self.super_key,
            command: self.ctrl,
        }
    }
}

pub fn key_code_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "$comment": "bevy_input::keyboard::KeyCode", "type": ["string", "object"] })
}

pub fn mouse_button_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "$comment": "bevy_input::mouse::MouseButton", "type": ["string", "object"] })
}

fn publish(map: Res<InputMap>, mut names: ResMut<KeyNames>) {
    *names = map.key_names();
}

/// The whole notches the wheel turned this frame, up counted above zero, and what a wheel that
/// scrolls smoothly has turned toward its next.
#[derive(Resource, Default)]
struct WheelNotches {
    turned: i32,
    toward_next: f32,
}

const PIXELS_PER_NOTCH: f32 = 60.0;

fn count_notches(scroll: Res<AccumulatedMouseScroll>, mut notches: ResMut<WheelNotches>) {
    let turned = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / PIXELS_PER_NOTCH,
    };
    let gathered = notches.toward_next + turned;
    notches.turned = gathered.trunc() as i32;
    notches.toward_next = gathered.fract();
}

const CTRL: [KeyCode; 2] = [KeyCode::ControlLeft, KeyCode::ControlRight];
const SHIFT: [KeyCode; 2] = [KeyCode::ShiftLeft, KeyCode::ShiftRight];
const ALT: [KeyCode; 2] = [KeyCode::AltLeft, KeyCode::AltRight];
const SUPER: [KeyCode; 2] = [KeyCode::SuperLeft, KeyCode::SuperRight];

fn pointer_button(button: MouseButton) -> Option<egui::PointerButton> {
    match button {
        MouseButton::Left => Some(egui::PointerButton::Primary),
        MouseButton::Right => Some(egui::PointerButton::Secondary),
        MouseButton::Middle => Some(egui::PointerButton::Middle),
        _ => None,
    }
}

fn held_names(words: &InputMessages, modifiers: KeyModifiers) -> Vec<String> {
    [
        (modifiers.ctrl, words.ctrl()),
        (modifiers.alt, words.alt()),
        (modifiers.shift, words.shift()),
        (modifiers.super_key, words.super_key()),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .map(|(_, name)| name.to_owned())
    .collect()
}

fn key_name(words: &InputMessages, key: KeyCode) -> String {
    let named = match key {
        KeyCode::Escape => words.escape(),
        KeyCode::Space => words.space(),
        KeyCode::Enter => words.enter(),
        KeyCode::Tab => words.tab(),
        KeyCode::Backspace => words.backspace(),
        KeyCode::Delete => words.delete(),
        KeyCode::Insert => words.insert(),
        KeyCode::Home => words.home(),
        KeyCode::End => words.end(),
        KeyCode::PageUp => words.page_up(),
        KeyCode::PageDown => words.page_down(),
        KeyCode::CapsLock => words.caps_lock(),
        KeyCode::ArrowUp => return "↑".to_owned(),
        KeyCode::ArrowDown => return "↓".to_owned(),
        KeyCode::ArrowLeft => return "←".to_owned(),
        KeyCode::ArrowRight => return "→".to_owned(),
        _ if CTRL.contains(&key) => words.ctrl(),
        _ if SHIFT.contains(&key) => words.shift(),
        _ if ALT.contains(&key) => words.alt(),
        _ if SUPER.contains(&key) => words.super_key(),
        _ => {
            let debug = format!("{key:?}");
            return ["Key", "Digit"]
                .iter()
                .find_map(|prefix| debug.strip_prefix(prefix))
                .unwrap_or(&debug)
                .to_owned();
        }
    };
    named.to_owned()
}

fn mouse_cap(
    words: &InputMessages,
    button: MouseButton,
    times: Clicks,
    drag: bool,
) -> (CapFace, String) {
    let presses = u8::from(times);
    let glyph = |glyph| CapFace::Mouse {
        glyph,
        presses,
        drag,
    };
    let gesture = match (drag, times) {
        (true, _) => "drag",
        (false, Clicks::Single) => "click",
        (false, Clicks::Double) => "double_click",
        (false, Clicks::Triple) => "triple_click",
    };
    let (shown, name) = match button {
        MouseButton::Left => (Some(MouseGlyph::Left), words.left(gesture)),
        MouseButton::Right => (Some(MouseGlyph::Right), words.right(gesture)),
        MouseButton::Middle => (Some(MouseGlyph::Middle), words.middle(gesture)),
        MouseButton::Back => (Some(MouseGlyph::Back), words.back(gesture)),
        MouseButton::Forward => (Some(MouseGlyph::Forward), words.forward(gesture)),
        MouseButton::Other(number) => (None, words.other(gesture, number)),
    };
    match shown {
        Some(shown) => (glyph(shown), name),
        None => (CapFace::Text(name.clone()), name),
    }
}
