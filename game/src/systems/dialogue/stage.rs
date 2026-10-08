use std::collections::HashMap;
use std::time::Duration;

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::tokens::palette;
use ui::{
    CastDepth, CastMember, ChipOptions, ChoiceOptions, DialogueBoxOptions, Family, RichPiece, Side,
};

use super::{
    ChoiceChip, ChoiceView, Conversation, ConversationRequest, ConversationStep, DialogueId,
    RefusedPick, Speaker, SpokenLine,
};
use crate::core::assets::AssetRef;
use crate::core::audio::playback::SfxId;
use crate::core::audio::playback::{PlaySfx, SfxPlace};
use crate::core::babble::{BabbleRank, Babbler, Babbling};
use crate::systems::actor::Name;
use crate::systems::actor::bust::{Busts, Face, GenericExpression};
use crate::systems::input::map::{self, ActionInput, InputAction, InputMap};
use crate::systems::item::card;
use crate::systems::npc::Npc;
use crate::systems::player;
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;
use crate::systems::scene::mode::Mode;

const READ_ALONG: Duration = Duration::from_millis(1800);
const CHOICE_SHORTCUTS: [InputAction; 9] = [
    InputAction::Choice1,
    InputAction::Choice2,
    InputAction::Choice3,
    InputAction::Choice4,
    InputAction::Choice5,
    InputAction::Choice6,
    InputAction::Choice7,
    InputAction::Choice8,
    InputAction::Choice9,
];
const CHECK: &str = "icons/misc/checkmark.png";
const LOCK: &str = "icons/cursors/lock001.png";

pub struct StagePlugin;

impl Plugin for StagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stage>()
            .add_systems(OnEnter(GameScene::Area), spawn_stage)
            .add_systems(
                OnExit(GameScene::Area),
                (
                    crate::systems::scene::despawn_all::<StageRoot>,
                    forget_stage,
                ),
            )
            .add_systems(
                Update,
                (
                    follow_conversation,
                    stage_keys
                        .run_if(not(ui::typing))
                        .run_if(resource_equals(Mode::Play)),
                    read_along.run_if(resource_equals(Mode::Spectate)),
                    track_typing,
                )
                    .chain()
                    .run_if(in_state(GameScene::Area)),
            )
            .add_observer(on_picked)
            .add_observer(on_refused);
    }
}

pub struct StageView {
    pub node: DialogueId,
    pub with: Option<Entity>,
    pub line: usize,
    pub lines: usize,
    pub speaker: Option<String>,
    pub text: Option<String>,
    pub typing: bool,
    pub choices: Vec<StageChoice>,
}

pub struct StageChoice {
    pub label: String,
    pub locked: bool,
}

pub fn view(world: &World) -> Option<StageView> {
    let shown = world.resource::<Stage>().shown.as_ref()?;
    Some(StageView {
        node: shown.node,
        with: shown.with,
        line: shown.line,
        lines: shown.lines.len(),
        speaker: shown
            .current()
            .and_then(|line| speaker_name(world, line.by)),
        text: shown
            .current()
            .map(|line| line.text.words(world.resource::<InputMap>())),
        typing: shown.typed_since.is_none(),
        choices: shown
            .choices
            .iter()
            .map(|choice| StageChoice {
                label: choice.label.words(world.resource::<InputMap>()),
                locked: choice.locked,
            })
            .collect(),
    })
}

pub fn show_panel(world: &mut World, panel: ui::DialoguePanelOptions) -> Option<Entity> {
    let slot = world
        .query_filtered::<Entity, With<StagePanels>>()
        .iter(world)
        .next()?;
    let mut spawned = world.spawn_scene(ui::dialogue_panel(panel)).ok()?;
    spawned.insert(ChildOf(slot));
    Some(spawned.id())
}

pub fn box_right(world: &mut World) -> Option<f32> {
    let dialogue = stage_box(world)?;
    let line = descendant_with::<ui::DialogueBox>(world, dialogue).unwrap_or(dialogue);
    Some(ui::node_rect(world, line)?.max.x)
}

pub fn rects(world: &mut World) -> Vec<Rect> {
    let Some(dialogue) = stage_box(world) else {
        return Vec::new();
    };
    let panels: Vec<Entity> = world
        .query_filtered::<&Children, With<StagePanels>>()
        .iter(world)
        .flat_map(|panels| panels.iter())
        .collect();
    std::iter::once(dialogue)
        .chain(panels)
        .filter_map(|shown| ui::node_rect(world, shown))
        .filter(|rect| rect.width() > 0.0 && rect.height() > 0.0)
        .collect()
}

pub fn leave(world: &mut World) -> bool {
    if *world.resource::<Mode>() != Mode::Play {
        return false;
    }
    let Some(step) = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .map(|shown| shown.step)
    else {
        return false;
    };
    world.write_message(ConversationRequest::Leave { step });
    true
}

#[derive(Resource, Default)]
struct Stage {
    shown: Option<Shown>,
}

struct Shown {
    step: ConversationStep,
    node: DialogueId,
    with: Option<Entity>,
    line: usize,
    lines: Vec<SpokenLine>,
    choices: Vec<ChoiceView>,
    picked: bool,
    typed_since: Option<Duration>,
    faces: HashMap<Speaker, Face>,
    stale: bool,
    remark: Option<SpokenLine>,
    remarks_seen: u32,
    refusals_seen: u32,
}

#[derive(Component, Default, Clone)]
struct StageRoot;

#[derive(Component, Default, Clone)]
struct StageBoxHost;

#[derive(Component, Default, Clone)]
struct StageBox;

#[derive(Component, Default, Clone)]
struct StagePanels;

fn spawn_stage(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        StageRoot
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
        }
        GlobalZIndex({ui::tokens::layer::STAGE})
        Pickable::IGNORE
        Children [
            {EntityScene(ui::cast())},
            (
                StageBoxHost
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    bottom: Val::Px(28.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                }
                Pickable::IGNORE
                Children [ (
                    StagePanels
                    Node {
                        width: Val::Percent(100.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::FlexEnd,
                        column_gap: Val::Px({ui::tokens::spacing::L}),
                        margin: {UiRect::bottom(Val::Px(ui::tokens::spacing::XL))},
                    }
                    Pickable::IGNORE
                ) ]
            ),
        ]
    });
}

fn forget_stage(mut stage: ResMut<Stage>) {
    stage.shown = None;
}

fn follow_conversation(world: &mut World) {
    let conversation = world
        .resource::<Viewpoint>()
        .0
        .and_then(|seen| world.get::<Conversation>(seen))
        .cloned();
    let Some(now) = conversation else {
        if world.resource::<Stage>().shown.is_some() {
            close(world);
        }
        return;
    };
    let mut stage = world.resource_mut::<Stage>();
    match stage.shown.as_mut() {
        None => {
            stage.shown = Some(Shown::of(now, HashMap::new()));
            chime(world, SfxId::UiOpen);
            show_line(world);
        }
        Some(shown) if shown.step != now.step => {
            let faces = std::mem::take(&mut shown.faces);
            stage.shown = Some(Shown::of(now, faces));
            show_line(world);
        }
        Some(shown) => {
            if shown.choices != now.choices {
                shown.stale = true;
            }
            let remark = now
                .remark
                .clone()
                .filter(|remark| remark.nth > shown.remarks_seen);
            let refused = now
                .refused
                .clone()
                .filter(|refused| refused.nth > shown.refusals_seen);
            shown.choices = now.choices;
            if let Some(refused) = refused {
                shown.refusals_seen = refused.nth;
                shown.picked = false;
                refuse(world, refused);
            }
            let mut stage = world.resource_mut::<Stage>();
            let Some(shown) = stage.shown.as_mut() else {
                return;
            };
            if let Some(remark) = remark {
                shown.remarks_seen = remark.nth;
                shown.remark = Some(remark.line);
                show_line(world);
            } else if shown.stale && shown.typed_since.is_some() {
                shown.stale = false;
                build_box(world, false);
            }
        }
    }
}

impl Shown {
    fn of(now: Conversation, faces: HashMap<Speaker, Face>) -> Shown {
        Shown {
            step: now.step,
            node: now.node,
            with: now.with,
            line: 0,
            lines: now.lines,
            choices: now.choices,
            picked: false,
            typed_since: None,
            faces,
            stale: false,
            remarks_seen: now.remark.map_or(0, |remark| remark.nth),
            refusals_seen: now.refused.map_or(0, |refused| refused.nth),
            remark: None,
        }
    }

    fn current(&self) -> Option<SpokenLine> {
        self.remark
            .clone()
            .or_else(|| self.lines.get(self.line).cloned())
    }

    fn last_line(&self) -> bool {
        self.line + 1 >= self.lines.len()
    }
}

fn close(world: &mut World) {
    world.resource_mut::<Stage>().shown = None;
    despawn_box(world);
    set_cast(world, Vec::new());
    chime(world, SfxId::UiClose);
}

fn show_line(world: &mut World) {
    let Some(line) = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .and_then(Shown::current)
    else {
        return;
    };
    let speaker_face = line.face;
    if let Some(shown) = world.resource_mut::<Stage>().shown.as_mut() {
        shown.typed_since = None;
        shown.stale = false;
        if let Some(face) = speaker_face {
            shown.faces.insert(line.by, face);
        }
    }
    let members = cast_members(world);
    set_cast(world, members);
    build_box(world, true);
}

fn advance(world: &mut World) {
    let Some(shown) = world.resource_mut::<Stage>().shown.as_mut().map(|shown| {
        let next = !shown.last_line();
        if next {
            shown.line += 1;
            shown.remark = None;
        }
        (next, shown.choices.is_empty(), shown.step)
    }) else {
        return;
    };
    match shown {
        (true, ..) => show_line(world),
        (false, true, step) if *world.resource::<Mode>() == Mode::Play => {
            world.write_message(ConversationRequest::Leave { step });
        }
        _ => {}
    }
}

fn stage_keys(world: &mut World) {
    if world.resource::<Stage>().shown.is_none() || ui::modal_open(world) {
        return;
    }
    match (typing_line(world), shown_choices(world)) {
        (Some(line), _) => {
            if map::just_pressed(world, InputAction::Advance)
                && let Some(mut typewriter) = world.get_mut::<ui::Typewriter>(line)
            {
                typewriter.finish();
            }
        }
        (None, Some(list)) => {
            if map::just_pressed(world, InputAction::PickChoice) {
                ui::pick_choice(world, list);
            } else if map::just_pressed(world, InputAction::ChoosePrevious) {
                ui::step_choice(world, list, -1);
                chime(world, SfxId::UiMove);
            } else if map::just_pressed(world, InputAction::ChooseNext) {
                ui::step_choice(world, list, 1);
                chime(world, SfxId::UiMove);
            } else if let Some(index) = map::just_pressed_index(world, &CHOICE_SHORTCUTS) {
                ui::pick_choice_at(world, list, index);
            }
        }
        (None, None) => {
            if map::just_pressed(world, InputAction::Advance) {
                advance(world);
            }
        }
    }
}

fn read_along(world: &mut World) {
    let now = world.resource::<Time>().elapsed();
    let due = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .is_some_and(|shown| {
            !shown.last_line()
                && shown
                    .typed_since
                    .is_some_and(|since| now.saturating_sub(since) >= READ_ALONG)
        });
    if due {
        advance(world);
    }
}

fn track_typing(world: &mut World) {
    let now = world.resource::<Time>().elapsed();
    let typing = typing_line(world).is_some();
    let built = stage_box(world).is_some();
    let mut stage = world.resource_mut::<Stage>();
    let Some(shown) = stage.shown.as_mut() else {
        return;
    };
    if built && !typing && shown.typed_since.is_none() {
        shown.typed_since = Some(now);
        if shown.stale {
            shown.stale = false;
            build_box(world, false);
        }
    }
}

fn on_picked(picked: On<ui::ChoicePicked>, mut commands: Commands) {
    let index = picked.index;
    commands.queue(move |world: &mut World| {
        if request_pick(world, index) {
            chime(world, SfxId::UiPick);
        }
    });
}

fn on_refused(refused: On<ui::ChoiceRefused>, mut commands: Commands) {
    let index = refused.index;
    commands.queue(move |world: &mut World| {
        request_pick(world, index);
    });
}

fn request_pick(world: &mut World, index: usize) -> bool {
    if *world.resource::<Mode>() != Mode::Play {
        return false;
    }
    let Some(step) = world
        .resource_mut::<Stage>()
        .shown
        .as_mut()
        .and_then(|shown| {
            shown.choices.get(index)?;
            (!std::mem::replace(&mut shown.picked, true)).then_some(shown.step)
        })
    else {
        return false;
    };
    world.write_message(ConversationRequest::Pick {
        step,
        choice: index as u32,
    });
    true
}

fn refuse(world: &mut World, refused: RefusedPick) {
    let index = refused.choice as usize;
    let shaken = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .and_then(|shown| shown.choices.get(index))
        .is_some_and(|choice| choice.locked);
    if !shaken && let Some(list) = shown_choices(world) {
        ui::shake_choice(world, list, index);
    }
}

fn advance_on_click(click: On<Pointer<Click>>, input: ActionInput, mut commands: Commands) {
    if !input.clicked(InputAction::Advance, &click) {
        return;
    }
    let target = click.original_event_target();
    let root = click.entity;
    commands.queue(move |world: &mut World| {
        let on_control = std::iter::successors(Some(target), |&entity| {
            (entity != root)
                .then(|| world.get::<ChildOf>(entity).map(ChildOf::parent))
                .flatten()
        })
        .any(|entity| {
            world.get::<ui::ChoiceRow>(entity).is_some()
                || world.get::<bevy::ui_widgets::Button>(entity).is_some()
        });
        let typed = world
            .resource::<Stage>()
            .shown
            .as_ref()
            .is_some_and(|shown| shown.typed_since.is_some());
        if !on_control && typed && *world.resource::<Mode>() == Mode::Play {
            advance(world);
        }
    });
}

fn build_box(world: &mut World, typed: bool) {
    let Some(host) = world
        .query_filtered::<Entity, With<StageBoxHost>>()
        .iter(world)
        .next()
    else {
        return;
    };
    let selected = shown_choices(world)
        .and_then(|list| world.get::<ui::ChoiceList>(list))
        .map(|list| list.selected);
    despawn_box(world);
    let Some(options) = box_options(world, typed) else {
        return;
    };
    let Ok(mut spawned) = world.spawn_scene(ui::dialogue_box(options)) else {
        return;
    };
    spawned.insert((StageBox, ChildOf(host)));
    spawned.observe(advance_on_click);
    if typed {
        babble_along(world);
    }
    if let (Some(selected), Some(list)) = (selected, shown_choices(world))
        && let Some(mut list) = world.get_mut::<ui::ChoiceList>(list)
    {
        list.selected = selected;
    }
}

fn box_options(world: &World, typed: bool) -> Option<DialogueBoxOptions> {
    let shown = world.resource::<Stage>().shown.as_ref()?;
    let line = shown.current()?;
    let assets = world.resource::<AssetServer>();
    let playing = *world.resource::<Mode>() == Mode::Play;
    let choices = if shown.last_line() {
        shown
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| choice_options(assets, choice, index))
            .collect()
    } else {
        Vec::new()
    };
    Some(DialogueBoxOptions {
        speaker: speaker_name(world, line.by).map(|name| (name, side_of(line.by))),
        line: line.text.rich(),
        typed,
        choices,
        hint: if playing {
            player_hint()
        } else {
            spectator_hint()
        },
        advance: InputAction::Advance.into(),
        pick: InputAction::PickChoice.into(),
    })
}

fn babble_along(world: &mut World) {
    let Some(by) = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .and_then(Shown::current)
        .map(|line| line.by)
    else {
        return;
    };
    let babble = match by {
        Speaker::Npc(npc) => npc.get().babble,
        Speaker::Player => world
            .resource::<Viewpoint>()
            .0
            .and_then(|seen| world.get::<Babbler>(seen))
            .map(|babbler| babbler.0),
        Speaker::Prop(_) | Speaker::Narrator => None,
    };
    if let (Some(babble), Some(line)) = (babble, typing_line(world)) {
        world.entity_mut(line).insert(Babbling {
            babble,
            place: SfxPlace::Interface,
            rank: BabbleRank::DIALOGUE_BOX,
        });
    }
}

fn player_hint() -> Vec<RichPiece> {
    vec![
        map::input(InputAction::ChoosePrevious),
        map::input(InputAction::ChooseNext),
        RichPiece::text(" choose · "),
        map::input(InputAction::PickChoice),
        RichPiece::text(" pick · "),
        map::input(InputAction::Choice1),
        RichPiece::text("–"),
        map::input(InputAction::Choice9),
        RichPiece::text(" shortcut · "),
        map::input(InputAction::Dismiss),
        RichPiece::text(" leave"),
    ]
}

fn spectator_hint() -> Vec<RichPiece> {
    vec![RichPiece::text("Watching")]
}

fn choice_options(assets: &AssetServer, choice: &ChoiceView, index: usize) -> ChoiceOptions {
    let mut chips: Vec<ChipOptions> = choice
        .chips
        .iter()
        .map(|chip| match chip {
            ChoiceChip::Needs { what, met } => ChipOptions {
                label: what.clone(),
                icon: Some(assets.load(if *met { CHECK } else { LOCK })),
                family: Family::outline(if *met {
                    palette::EMERALD_70
                } else {
                    palette::CRIMSON_70
                }),
                inspect: None,
            },
            ChoiceChip::Pays { item, count, have } => ChipOptions {
                label: count.to_string(),
                icon: Some(assets.load(item.get().icon.0)),
                family: Family::outline(if have >= count {
                    palette::AMBER_70
                } else {
                    palette::CRIMSON_70
                }),
                inspect: None,
            },
            ChoiceChip::Gets { item, count } => ChipOptions {
                label: format!("+{count}"),
                icon: Some(assets.load(item.get().icon.0)),
                family: Family::outline(palette::EMERALD_70),
                inspect: Some(card::inspectable(*item)),
            },
        })
        .collect();
    if let Some(warn) = &choice.warn {
        chips.push(ChipOptions {
            label: warn.clone(),
            icon: None,
            family: Family::outline(palette::CRIMSON_80),
            inspect: None,
        });
    }
    ChoiceOptions {
        label: choice.label.rich(),
        icon: choice.icon.as_ref().map(|icon| assets.load(icon.clone())),
        chips,
        locked: choice.locked,
        shortcut: CHOICE_SHORTCUTS.get(index).map(|&action| action.into()),
    }
}

fn cast_members(world: &World) -> Vec<CastMember> {
    let Some(shown) = world.resource::<Stage>().shown.as_ref() else {
        return Vec::new();
    };
    let Some(current) = shown.current() else {
        return Vec::new();
    };
    let mut speakers = vec![current.by];
    if shown.remark.is_none() {
        speakers.push(Speaker::Player);
        if let Some(npc) = shown.with.and_then(|with| world.get::<Npc>(with)) {
            speakers.push(Speaker::Npc(npc.def));
        }
        for line in shown.lines.iter().take(shown.line + 1) {
            speakers.push(line.by);
        }
    }
    let mut seen = std::collections::HashSet::new();
    speakers.retain(|who| seen.insert(*who));
    speakers.retain(|&who| busts_of(who).is_some());
    let assets = world.resource::<AssetServer>();
    let fronts: Vec<Speaker> = [Side::Left, Side::Right]
        .into_iter()
        .filter_map(|side| {
            let mut standing = speakers.iter().filter(|&&who| side_of(who) == side);
            standing
                .clone()
                .find(|&&who| who == current.by)
                .or_else(|| standing.next())
                .copied()
        })
        .collect();
    speakers
        .into_iter()
        .filter_map(|who| {
            let busts = busts_of(who)?;
            let face = shown
                .faces
                .get(&who)
                .copied()
                .unwrap_or(Face::Generic(GenericExpression::Neutral));
            let bust = busts
                .face(face)
                .unwrap_or_else(|| busts.generic.of(GenericExpression::Neutral));
            let lit = who == current.by;
            Some(CastMember {
                key: cast_key(who),
                image: bust_image(assets, bust),
                side: side_of(who),
                depth: if fronts.contains(&who) {
                    CastDepth::Front
                } else {
                    CastDepth::Back
                },
                lit,
                flip: false,
            })
        })
        .collect()
}

fn set_cast(world: &mut World, members: Vec<CastMember>) {
    let mut casts = world.query::<&mut ui::Cast>();
    for mut cast in casts.iter_mut(world) {
        if cast.members != members {
            cast.members = members.clone();
        }
    }
}

fn bust_image(assets: &AssetServer, art: AssetRef) -> Handle<Image> {
    assets.load(art.0)
}

fn busts_of(who: Speaker) -> Option<&'static Busts> {
    match who {
        Speaker::Npc(npc) => npc.get().model.get().busts.as_ref(),
        Speaker::Player => player::def().model.get().busts.as_ref(),
        Speaker::Prop(_) | Speaker::Narrator => None,
    }
}

fn speaker_name(world: &World, who: Speaker) -> Option<String> {
    match who {
        Speaker::Npc(npc) => Some(npc.get().display_name.to_owned()),
        Speaker::Prop(prop) => Some(prop.get().display_name.to_owned()),
        Speaker::Player => world
            .resource::<Viewpoint>()
            .0
            .and_then(|seen| world.get::<Name>(seen))
            .map(|name| name.name.clone()),
        Speaker::Narrator => None,
    }
}

fn side_of(who: Speaker) -> Side {
    match who {
        Speaker::Player => Side::Left,
        Speaker::Npc(_) | Speaker::Prop(_) | Speaker::Narrator => Side::Right,
    }
}

fn cast_key(who: Speaker) -> u64 {
    match who {
        Speaker::Player => 0,
        Speaker::Narrator => 1,
        Speaker::Npc(npc) => 2 + npc.index() as u64,
        Speaker::Prop(prop) => 10_000 + prop.index() as u64,
    }
}

fn chime(world: &mut World, id: SfxId) {
    world.write_message(PlaySfx {
        id,
        place: SfxPlace::Interface,
    });
}

fn stage_box(world: &mut World) -> Option<Entity> {
    world
        .query_filtered::<Entity, With<StageBox>>()
        .iter(world)
        .next()
}

fn despawn_box(world: &mut World) {
    if let Some(dialogue) = stage_box(world) {
        world.entity_mut(dialogue).despawn();
    }
}

fn typing_line(world: &mut World) -> Option<Entity> {
    let dialogue = stage_box(world)?;
    ui::dialogue_typing(world, dialogue)
}

fn shown_choices(world: &mut World) -> Option<Entity> {
    let dialogue = stage_box(world)?;
    let last = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .is_some_and(Shown::last_line);
    let list = ui::dialogue_choices(world, dialogue)?;
    let rows = world
        .get::<Children>(list)
        .is_some_and(|rows| !rows.is_empty());
    (last && rows).then_some(list)
}

fn descendant_with<C: Component>(world: &World, root: Entity) -> Option<Entity> {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if world.get::<C>(entity).is_some() {
            return Some(entity);
        }
        if let Some(kids) = world.get::<Children>(entity) {
            stack.extend(kids.iter());
        }
    }
    None
}
