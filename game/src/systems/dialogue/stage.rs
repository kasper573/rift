use std::collections::HashMap;
use std::time::Duration;

use bevy::image::{ImageLoaderSettings, ImageSampler};
use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::tokens::palette;
use ui::{CastDepth, CastMember, ChipOptions, ChoiceOptions, DialogueBoxOptions, Family, Side};

use super::history::{self, HistoryEntry};
use super::{
    ChoiceChip, ChoiceView, Conversation, ConversationRequest, ConversationStep, DialogueId,
    Speaker, SpokenLine, WaitingView,
};
use crate::core::assets::AssetRef;
use crate::core::sfx::SfxId;
use crate::core::sfx::playback::{PlaySfx, SfxPlace};
use crate::systems::actor::Name;
use crate::systems::actor::bust::{Busts, Face, GenericExpression};
use crate::systems::item::card;
use crate::systems::npc::Npc;
use crate::systems::player;
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;
use crate::systems::scene::mode::Mode;

const READ_ALONG: Duration = Duration::from_millis(1800);
const PLAYER_HINT: &str = "↑ ↓ choose · Enter pick · 1-9 shortcut · H history · Esc leave";
const SPECTATOR_HINT: &str = "Watching · H history";
const CHECK: &str = "icons/misc/checkmark.png";
const LOCK: &str = "icons/cursors/lock001.png";
const SANDCLOCK: &str = "icons/cursors/sandclock001.png";

pub struct StagePlugin;

impl Plugin for StagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stage>()
            .add_plugins(history::HistoryPlugin)
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
                    count_down,
                )
                    .chain()
                    .run_if(in_state(GameScene::Area))
                    .before(ui::UiReactive),
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
    pub typing: bool,
    pub choices: Vec<StageChoice>,
    pub waiting: Option<DialogueId>,
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
        lines: shown.node.get().lines.len(),
        speaker: shown
            .current()
            .and_then(|line| speaker_name(world, line.by)),
        typing: shown.typed_since.is_none(),
        choices: shown
            .choices
            .iter()
            .map(|choice| StageChoice {
                label: choice.label.words(),
                locked: choice.refusal.is_some(),
            })
            .collect(),
        waiting: shown.waiting.map(|waiting| waiting.node),
    })
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
    choices: Vec<ChoiceView>,
    waiting: Option<WaitingView>,
    waiting_until: Duration,
    refusal: Option<String>,
    picked: bool,
    typed_since: Option<Duration>,
    faces: HashMap<Speaker, Face>,
    stale: bool,
    remark: Option<SpokenLine>,
    remarks_seen: u32,
}

#[derive(Component, Default, Clone)]
struct StageRoot;

#[derive(Component, Default, Clone)]
struct StageBoxHost;

#[derive(Component, Default, Clone)]
struct StageBox;

#[derive(Component, Default, Clone)]
struct WaitingChip;

fn spawn_stage(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        StageRoot
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
        }
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
                    justify_content: JustifyContent::Center,
                }
                Pickable::IGNORE
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
    let time = world.resource::<Time>().elapsed();
    let mut stage = world.resource_mut::<Stage>();
    match stage.shown.as_mut() {
        None => {
            stage.shown = Some(Shown::of(now, HashMap::new(), time));
            chime(world, SfxId::UiOpen);
            let with = world
                .resource::<Stage>()
                .shown
                .as_ref()
                .and_then(|shown| shown.with)
                .and_then(|with| world.get::<Name>(with))
                .map(|name| name.name.clone());
            if let Some(with) = with {
                history::record(world, HistoryEntry::Began(with));
            }
            show_line(world);
        }
        Some(shown) if shown.step != now.step => {
            let faces = std::mem::take(&mut shown.faces);
            stage.shown = Some(Shown::of(now, faces, time));
            show_line(world);
        }
        Some(shown) => {
            let waiting_changed = shown.waiting.map(|waiting| waiting.node)
                != now.waiting.map(|waiting| waiting.node);
            if shown.choices != now.choices || waiting_changed {
                shown.stale = true;
            }
            let remark = now
                .remark
                .clone()
                .filter(|remark| remark.nth > shown.remarks_seen);
            if waiting_changed && now.waiting.is_some() {
                shown.stale = true;
                chime(world, SfxId::UiChime);
            }
            let mut stage = world.resource_mut::<Stage>();
            let Some(shown) = stage.shown.as_mut() else {
                return;
            };
            if shown.waiting.map(|waiting| waiting.lasts)
                != now.waiting.map(|waiting| waiting.lasts)
            {
                shown.waiting_until = time + lasting(now.waiting);
            }
            shown.choices = now.choices;
            shown.waiting = now.waiting;
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
    fn of(now: Conversation, faces: HashMap<Speaker, Face>, time: Duration) -> Shown {
        Shown {
            step: now.step,
            node: now.node,
            with: now.with,
            line: 0,
            choices: now.choices,
            waiting_until: time + lasting(now.waiting),
            waiting: now.waiting,
            refusal: None,
            picked: false,
            typed_since: None,
            faces,
            stale: false,
            remarks_seen: now.remark.map_or(0, |remark| remark.nth),
            remark: None,
        }
    }

    fn current(&self) -> Option<SpokenLine> {
        self.remark
            .clone()
            .or_else(|| self.node.get().lines.get(self.line).map(SpokenLine::of))
    }

    fn last_line(&self) -> bool {
        self.line + 1 >= self.node.get().lines.len()
    }
}

fn lasting(waiting: Option<WaitingView>) -> Duration {
    waiting.map_or(Duration::ZERO, |waiting| {
        Duration::from_secs_f32(waiting.lasts.0.max(0.0))
    })
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
        shown.refusal = None;
        shown.stale = false;
        if let Some(face) = speaker_face {
            shown.faces.insert(line.by, face);
        }
    }
    if line.cue
        && let Some(cue) = speaker_face
            .and_then(|face| busts_of(line.by).and_then(|busts| busts.face(face)))
            .map(|bust| bust.cue)
    {
        chime(world, cue);
    }
    history::record(
        world,
        HistoryEntry::Said {
            who: speaker_name(world, line.by),
            text: line.text,
        },
    );
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
    let pressed: Vec<KeyCode> = world
        .resource::<ButtonInput<KeyCode>>()
        .get_just_pressed()
        .copied()
        .collect();
    for key in pressed {
        let typing = typing_line(world);
        let choices = shown_choices(world);
        match (key, typing, choices) {
            (KeyCode::Enter | KeyCode::Space, Some(line), _) => {
                if let Some(mut typewriter) = world.get_mut::<ui::Typewriter>(line) {
                    typewriter.finish();
                }
            }
            (KeyCode::Enter, None, Some(list)) => ui::pick_choice(world, list),
            (KeyCode::Enter | KeyCode::Space, None, None) => advance(world),
            (KeyCode::ArrowUp, None, Some(list)) => {
                ui::step_choice(world, list, -1);
                chime(world, SfxId::UiMove);
            }
            (KeyCode::ArrowDown, None, Some(list)) => {
                ui::step_choice(world, list, 1);
                chime(world, SfxId::UiMove);
            }
            (key, None, Some(list)) => {
                if let Some(index) = digit(key) {
                    ui::pick_choice_at(world, list, index);
                }
            }
            _ => {}
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

fn count_down(world: &mut World) {
    let now = world.resource::<Time>().elapsed();
    let Some((node, until)) = world.resource::<Stage>().shown.as_ref().and_then(|shown| {
        shown
            .waiting
            .map(|waiting| (waiting.node, shown.waiting_until))
    }) else {
        return;
    };
    let label = waiting_label(node, until.saturating_sub(now));
    let chips: Vec<Entity> = world
        .query_filtered::<Entity, With<WaitingChip>>()
        .iter(world)
        .collect();
    for chip in chips {
        if let Some(text) = descendant_with::<Text>(world, chip)
            && let Some(mut text) = world.get_mut::<Text>(text)
            && text.0 != label
        {
            text.0 = label.clone();
        }
    }
}

fn on_picked(picked: On<ui::ChoicePicked>, mut commands: Commands) {
    let index = picked.index;
    commands.queue(move |world: &mut World| {
        if *world.resource::<Mode>() != Mode::Play {
            return;
        }
        let Some((step, label)) = world
            .resource_mut::<Stage>()
            .shown
            .as_mut()
            .and_then(|shown| {
                let label = shown.choices.get(index)?.label.clone();
                (!std::mem::replace(&mut shown.picked, true)).then_some((shown.step, label))
            })
        else {
            return;
        };
        world.write_message(ConversationRequest::Pick {
            step,
            choice: index as u32,
        });
        chime(world, SfxId::UiPick);
        history::record(world, HistoryEntry::Picked(label));
    });
}

fn on_refused(refused: On<ui::ChoiceRefused>, mut commands: Commands) {
    let index = refused.index;
    commands.queue(move |world: &mut World| {
        let mut stage = world.resource_mut::<Stage>();
        let Some(shown) = stage.shown.as_mut() else {
            return;
        };
        shown.refusal = shown
            .choices
            .get(index)
            .and_then(|choice| choice.refusal.clone());
        let refusal = shown.refusal.clone();
        chime(world, SfxId::UiRefuse);
        if let Some(dialogue) = stage_box(world) {
            ui::set_dialogue_status(world, dialogue, refusal);
        }
    });
}

fn advance_on_click(click: On<Pointer<Click>>, mut commands: Commands) {
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
            .map(|choice| choice_options(assets, choice))
            .collect()
    } else {
        Vec::new()
    };
    let mut actions: Vec<Box<dyn Scene>> = Vec::new();
    if let Some(waiting) = shown.waiting {
        actions.push(Box::new(waiting_chip(
            assets,
            waiting_label(waiting.node, lasting(Some(waiting))),
        )));
    }
    actions.push(Box::new(frame_button("History", history::toggle)));
    if playing {
        actions.push(Box::new(frame_button("Leave", |world| {
            leave(world);
        })));
    }
    Some(DialogueBoxOptions {
        speaker: speaker_name(world, line.by).map(|name| (name, side_of(line.by))),
        line: line.text.rich(),
        typed,
        choices,
        hint: if playing { PLAYER_HINT } else { SPECTATOR_HINT }.to_owned(),
        actions,
        status: shown.refusal.clone(),
    })
}

fn choice_options(assets: &AssetServer, choice: &ChoiceView) -> ChoiceOptions {
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
                link: None,
            },
            ChoiceChip::Pays { item, count, have } => ChipOptions {
                label: count.to_string(),
                icon: Some(assets.load(item.get().icon.0)),
                family: Family::outline(if have >= count {
                    palette::AMBER_70
                } else {
                    palette::CRIMSON_70
                }),
                link: None,
            },
            ChoiceChip::Gets { item, count } => ChipOptions {
                label: format!("+{count}"),
                icon: Some(assets.load(item.get().icon.0)),
                family: Family::outline(palette::EMERALD_70),
                link: Some(card::link(*item)),
            },
        })
        .collect();
    if let Some(warn) = &choice.warn {
        chips.push(ChipOptions {
            label: warn.clone(),
            icon: None,
            family: Family::outline(palette::CRIMSON_80),
            link: None,
        });
    }
    ChoiceOptions {
        label: choice.label.rich(),
        icon: choice.icon.as_ref().map(|icon| assets.load(icon.clone())),
        chips,
        locked: choice.refusal.is_some(),
    }
}

fn frame_button(label: &'static str, act: fn(&mut World)) -> impl Scene {
    bsn! {
        {ui::button_styled(ui::button::intent::SECONDARY, ui::ButtonSize::Sm, label)}
        on(move |_: On<ui::Activate>, mut commands: Commands| {
            commands.queue(move |world: &mut World| act(world));
        })
    }
}

fn waiting_chip(assets: &AssetServer, label: String) -> impl Scene {
    let chip = ui::chip(ChipOptions {
        label,
        icon: Some(assets.load(SANDCLOCK)),
        family: Family {
            base: ui::theme::theme().surface_trough.base,
            ..Family::outline(palette::AMBER_70)
        },
        link: None,
    });
    bsn! {
        {chip}
        WaitingChip
    }
}

fn waiting_label(node: DialogueId, left: Duration) -> String {
    let who = node
        .get()
        .lines
        .iter()
        .find_map(|line| match line.by {
            Speaker::Npc(npc) => Some(npc.get().display_name),
            Speaker::Prop(prop) => Some(prop.get().display_name),
            Speaker::Player | Speaker::Narrator => None,
        })
        .unwrap_or("Someone");
    let seconds = left.as_secs_f32().ceil() as u32;
    format!("Next: {who} · {}:{:02}", seconds / 60, seconds % 60)
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
        for line in &shown.node.get().lines[..=shown.line] {
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
                image: bust_image(assets, bust.art),
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
    assets
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| {
            settings.sampler = ImageSampler::linear();
        })
        .load(art.0)
}

fn busts_of(who: Speaker) -> Option<&'static Busts> {
    match who {
        Speaker::Npc(npc) => npc.get().model.get().busts.as_ref(),
        Speaker::Player => player::MODEL.get().busts.as_ref(),
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
        Speaker::Npc(npc) => 2 + npc as u64,
        Speaker::Prop(prop) => 10_000 + prop as u64,
    }
}

fn digit(key: KeyCode) -> Option<usize> {
    [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ]
    .iter()
    .position(|digit| *digit == key)
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
