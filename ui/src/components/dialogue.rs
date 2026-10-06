use std::time::Duration;

use bevy::image::Image;
use bevy_asset::Handle;
use bevy_color::{Alpha, Color};
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::prelude::*;
use bevy_picking::prelude::{Click, Over, Pickable, Pointer};
use bevy_scene::{EntityScene, Scene, bsn, on, template_value};
use bevy_time::Time;
use bevy_ui::widget::ImageNode;
use bevy_ui::{
    AlignItems, BackgroundColor, BorderColor, BorderRadius, Checked, Display, FlexDirection,
    JustifyContent, Node, Overflow, PositionType, UiRect, UiTransform, Val, Val2,
};

use crate::Side;
use crate::component;
use crate::components::chip::{ChipOptions, chip, key_hint};
use crate::components::input::{CatalogInput, InputRef, input_cap};
use crate::components::rich_text::{RichPiece, RichText, Typewriter, rich_text};
use crate::components::text::styled_text;
use crate::cursor::CursorStyle;
use crate::motion::transition::STANDARD_ENTER;
use crate::state::ancestor_with;
use crate::style::Style;
use crate::theme::theme;
use crate::tokens::{palette, radius, spacing, typography};

const BOX_WIDTH: f32 = 920.0;
const ICON: f32 = 18.0;
const SHAKE: Duration = Duration::from_millis(320);

pub struct DialogueBoxOptions {
    pub speaker: Option<(String, Side)>,
    pub line: RichText,
    pub typed: bool,
    pub choices: Vec<ChoiceOptions>,
    pub hint: Vec<RichPiece>,
    pub actions: Vec<Box<dyn Scene>>,
    pub status: Option<String>,
    pub advance: InputRef,
    pub pick: InputRef,
}

pub struct DialoguePanelOptions {
    pub title: String,
    pub width: Val,
    pub height: Val,
    pub content: Box<dyn Scene>,
}

#[derive(Clone)]
pub struct ChoiceOptions {
    pub label: RichText,
    pub icon: Option<Handle<Image>>,
    pub chips: Vec<ChipOptions>,
    pub locked: bool,
    pub shortcut: Option<InputRef>,
}

#[derive(Component, Clone)]
#[require(Node)]
pub struct DialogueBox {
    pub advance: InputRef,
}

#[derive(Component, Clone, Default)]
pub(crate) struct DialogueLine;

#[derive(Component, Clone, Default)]
pub(crate) struct DialogueContinue;

#[derive(Component, Clone, Default)]
pub(crate) struct DialogueStatus;

#[derive(Component, Clone)]
#[require(Node)]
pub struct ChoiceList {
    pub selected: usize,
    pub pick: InputRef,
}

#[derive(Component, Clone, Default)]
#[require(Node, CursorStyle::Pointer)]
pub struct ChoiceRow {
    pub index: usize,
    pub locked: bool,
}

#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct ChoicePicked {
    #[event_target]
    pub list: Entity,
    pub index: usize,
}

#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct ChoiceRefused {
    #[event_target]
    pub list: Entity,
    pub index: usize,
}

#[derive(Component)]
pub(crate) struct Shake(Duration);

pub fn dialogue_box(options: DialogueBoxOptions) -> impl Scene {
    let DialogueBoxOptions {
        speaker,
        line,
        typed,
        choices,
        hint,
        actions,
        status,
        advance,
        pick,
    } = options;
    let surface = theme().surface_trough;
    let frame = Style::new()
        .background(surface.base.with_alpha(0.94))
        .border_color(edge(surface.on))
        .node(|node| {
            node.position_type = PositionType::Relative;
            node.flex_direction = FlexDirection::Column;
            node.row_gap = Val::Px(spacing::XL);
            node.width = Val::Px(BOX_WIDTH);
            node.max_width = Val::Vw(94.0);
            node.min_height = Val::Px(150.0);
            node.padding = UiRect::new(
                Val::Px(spacing::XXL + 4.0),
                Val::Px(spacing::XXL + 4.0),
                Val::Px(spacing::XXL),
                Val::Px(spacing::XXL),
            );
            node.border = UiRect::all(Val::Px(2.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::M));
        });
    let plate_side = speaker.as_ref().map(|(_, side)| *side);
    let plate = speaker.map(|(name, side)| EntityScene(name_plate(name, side)));
    let actions = (!actions.is_empty()).then(|| {
        let mut node = Node {
            position_type: PositionType::Absolute,
            top: Val::Px(-16.0),
            column_gap: Val::Px(spacing::M),
            align_items: AlignItems::Center,
            ..Node::default()
        };
        match plate_side {
            Some(Side::Left) => node.right = Val::Px(spacing::XL),
            _ => node.left = Val::Px(spacing::XL),
        }
        bsn! {
            template_value(node)
            Children [ {actions} ]
        }
    });
    let status_display = if status.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    let status = bsn! {
        DialogueStatus
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            bottom: Val::Px({spacing::XXXL}),
            justify_content: JustifyContent::Center,
            display: {status_display},
        }
        Children [ {EntityScene(styled_text(status.unwrap_or_default(), palette::CRIMSON_80, typography::BODY))} ]
    };
    let line = RichText {
        size: typography::LINE.font_size,
        ..line
    };
    let continue_mark = styled_text("↓", surface.on, typography::HINT);
    bsn! {
        component(DialogueBox { advance })
        template_value(frame)
        Pickable { should_block_lower: true, is_hoverable: true }
        on(finish_on_click)
        Children [
            {plate},
            {actions},
            ( {rich_text(line, typed)} DialogueLine ),
            {EntityScene(choice_list(choices, pick))},
            (
                DialogueContinue
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px({spacing::XXL}),
                    bottom: Val::Px({spacing::L}),
                    display: Display::None,
                }
                Children [ {EntityScene(continue_mark)} ]
            ),
            {status},
            (
                Node { position_type: PositionType::Absolute, bottom: Val::Px(-12.0), left: Val::Px({spacing::XL}) }
                Children [ {EntityScene(key_hint(hint, theme().surface_trough))} ]
            ),
        ]
    }
}

pub fn dialogue_panel(options: DialoguePanelOptions) -> impl Scene {
    let DialoguePanelOptions {
        title,
        width,
        height,
        content,
    } = options;
    let surface = theme().surface_floating;
    let header = theme().surface_inset;
    let frame = Node {
        width,
        height,
        max_width: Val::Vw(94.0),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(radius::M)),
        flex_direction: FlexDirection::Column,
        overflow: Overflow::clip(),
        ..Node::default()
    };
    bsn! {
        template_value(frame)
        BackgroundColor({surface.base})
        component(BorderColor::all(surface.border))
        Children [
            (
                Node { width: Val::Percent(100.0), padding: {UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::M))} }
                BackgroundColor({header.base})
                Children [ {EntityScene(styled_text(title, header.on, typography::BODY))} ]
            ),
            (
                Node { flex_grow: 1.0, min_height: Val::Px(0.0) }
                Children [ (
                    Node { width: Val::Percent(100.0), height: Val::Percent(100.0) }
                    Children [ {EntityScene(content)} ]
                ) ]
            ),
        ]
    }
}

pub fn choice_list(choices: Vec<ChoiceOptions>, pick: InputRef) -> impl Scene {
    let selected = choices
        .iter()
        .position(|choice| !choice.locked)
        .unwrap_or_default();
    let rows: Vec<Box<dyn Scene>> = choices
        .into_iter()
        .enumerate()
        .map(|(index, choice)| -> Box<dyn Scene> { Box::new(choice_row(index, choice)) })
        .collect();
    bsn! {
        component(ChoiceList { selected, pick })
        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(2.0) }
        Children [ {rows} ]
    }
}

pub fn step_choice(world: &mut World, list: Entity, delta: i32) {
    let rows = row_count(world, list);
    if rows == 0 {
        return;
    }
    if let Some(mut choices) = world.get_mut::<ChoiceList>(list) {
        let next = (choices.selected as i32 + delta).rem_euclid(rows as i32);
        choices.selected = next as usize;
    }
}

pub fn pick_choice(world: &mut World, list: Entity) {
    if let Some(selected) = world
        .get::<ChoiceList>(list)
        .map(|choices| choices.selected)
    {
        pick_choice_at(world, list, selected);
    }
}

pub fn pick_choice_at(world: &mut World, list: Entity, index: usize) {
    let Some(row) = rows(world, list).into_iter().find(|row| {
        world
            .get::<ChoiceRow>(*row)
            .is_some_and(|choice| choice.index == index)
    }) else {
        return;
    };
    if let Some(mut choices) = world.get_mut::<ChoiceList>(list) {
        choices.selected = index;
    }
    let locked = world
        .get::<ChoiceRow>(row)
        .is_some_and(|choice| choice.locked);
    if locked {
        let now = world.resource::<Time>().elapsed();
        world.entity_mut(row).insert(Shake(now));
        world.trigger(ChoiceRefused { list, index });
    } else {
        world.trigger(ChoicePicked { list, index });
    }
}

pub fn dialogue_choices(world: &World, dialogue: Entity) -> Option<Entity> {
    descendant_with::<ChoiceList>(world, dialogue)
}

pub fn set_dialogue_status(world: &mut World, dialogue: Entity, status: Option<String>) {
    let Some(holder) = descendant_with::<DialogueStatus>(world, dialogue) else {
        return;
    };
    if let Some(mut node) = world.get_mut::<Node>(holder) {
        node.display = if status.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Some(text) = descendant_with::<bevy_ui::widget::Text>(world, holder)
        && let Some(mut text) = world.get_mut::<bevy_ui::widget::Text>(text)
    {
        text.0 = status.unwrap_or_default();
    }
}

pub fn dialogue_typing(world: &World, dialogue: Entity) -> Option<Entity> {
    descendant_with::<DialogueLine>(world, dialogue).filter(|line| {
        world
            .get::<Typewriter>(*line)
            .is_some_and(|typewriter| !typewriter.is_finished())
    })
}

pub(crate) fn reveal_choices(
    boxes: Query<Entity, With<DialogueBox>>,
    children: Query<&Children>,
    lines: Query<&Typewriter, With<DialogueLine>>,
    mut lists: Query<&mut Node, (With<ChoiceList>, Without<DialogueContinue>)>,
    mut marks: Query<&mut Node, (With<DialogueContinue>, Without<ChoiceList>)>,
) {
    for dialogue in &boxes {
        let Ok(kids) = children.get(dialogue) else {
            continue;
        };
        let typed = kids
            .iter()
            .find_map(|kid| lines.get(kid).ok())
            .is_none_or(Typewriter::is_finished);
        let mut has_choices = false;
        for kid in kids.iter() {
            if let Ok(mut node) = lists.get_mut(kid) {
                has_choices = children.get(kid).is_ok_and(|rows| !rows.is_empty());
                let display = if typed { Display::Flex } else { Display::None };
                if node.display != display {
                    node.display = display;
                }
            }
        }
        for kid in kids.iter() {
            if let Ok(mut node) = marks.get_mut(kid) {
                let display = if typed && !has_choices {
                    Display::Flex
                } else {
                    Display::None
                };
                if node.display != display {
                    node.display = display;
                }
            }
        }
    }
}

pub(crate) fn mark_selected_choice(
    lists: Query<(&ChoiceList, &Children)>,
    rows: Query<(Entity, &ChoiceRow, Has<Checked>)>,
    mut commands: Commands,
) {
    for (list, kids) in &lists {
        for (row, choice, checked) in rows.iter_many(kids) {
            let selected = choice.index == list.selected;
            if selected && !checked {
                commands.entity(row).insert(Checked);
            } else if !selected && checked {
                commands.entity(row).remove::<Checked>();
            }
        }
    }
}

pub(crate) fn shake_refused(
    time: Res<Time>,
    mut commands: Commands,
    mut rows: Query<(Entity, &Shake, &mut UiTransform)>,
) {
    let now = time.elapsed();
    for (row, shake, mut transform) in &mut rows {
        let age = now.saturating_sub(shake.0);
        if age >= SHAKE {
            transform.translation = Val2::ZERO;
            commands.entity(row).remove::<Shake>();
            continue;
        }
        let fade = 1.0 - age.as_secs_f32() / SHAKE.as_secs_f32();
        let offset = (age.as_secs_f32() * 70.0).sin() * 6.0 * fade;
        transform.translation = Val2::px(offset, 0.0);
    }
}

fn finish_on_click(
    click: On<Pointer<Click>>,
    boxes: Query<&DialogueBox>,
    gestures: CatalogInput,
    mut commands: Commands,
) {
    let dialogue = click.entity;
    if !boxes
        .get(dialogue)
        .is_ok_and(|dialogue| gestures.clicked(dialogue.advance, &click))
    {
        return;
    }
    commands.queue(move |world: &mut World| {
        if let Some(line) = dialogue_typing(world, dialogue)
            && let Some(mut typewriter) = world.get_mut::<Typewriter>(line)
        {
            typewriter.finish();
        }
    });
}

fn name_plate(name: String, side: Side) -> impl Scene {
    let surface = theme().surface_trough;
    let inset = Val::Px(spacing::XXXL);
    let style = Style::new()
        .background(surface.base)
        .border_color(edge(surface.on))
        .node(move |node| {
            node.position_type = PositionType::Absolute;
            node.top = Val::Px(-22.0);
            match side {
                Side::Left => node.left = inset,
                _ => node.right = inset,
            }
            node.padding = UiRect::axes(Val::Px(spacing::XL), Val::Px(spacing::M));
            node.border = UiRect::all(Val::Px(2.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
        });
    bsn! {
        template_value(style)
        Children [ {EntityScene(styled_text(name, surface.on, typography::NAME))} ]
    }
}

fn choice_row(index: usize, choice: ChoiceOptions) -> impl Scene {
    let ChoiceOptions {
        label,
        icon,
        chips,
        locked,
        shortcut,
    } = choice;
    let selected = palette::AMBER_70;
    let style = Style::new()
        .background(Color::NONE)
        .border_color(Color::NONE)
        .node(|node| {
            node.align_items = AlignItems::Center;
            node.column_gap = Val::Px(spacing::L);
            node.padding = UiRect::axes(Val::Px(spacing::L), Val::Px(2.0));
            node.border = UiRect::all(Val::Px(1.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
        })
        .hover(Style::new().background(selected.with_alpha(0.08)))
        .checked(
            Style::new()
                .background(selected.with_alpha(0.14))
                .border_color(selected.with_alpha(0.7)),
        )
        .transition(STANDARD_ENTER);
    let ink = theme().surface_trough.on;
    let label_color = if locked { ink.with_alpha(0.45) } else { ink };
    let label = RichText {
        size: typography::BODY.font_size + 1.0,
        color: label_color,
        ..label
    };
    let shortcut = shortcut.map(|input| {
        EntityScene(input_cap(
            input,
            typography::BODY.font_size,
            ink.with_alpha(0.7),
        ))
    });
    let icon = icon.map(|image| {
        bsn! {
            Node { width: Val::Px(ICON), height: Val::Px(ICON) }
            component(ImageNode::new(image))
        }
    });
    let chips: Vec<Box<dyn Scene>> = chips
        .into_iter()
        .map(|options| -> Box<dyn Scene> { Box::new(chip(options)) })
        .collect();
    bsn! {
        template_value(style)
        ChoiceRow { index: {index}, locked: {locked} }
        UiTransform
        on(pick_on_click)
        on(select_on_hover)
        Children [
            {shortcut},
            {icon},
            ( Node { flex_grow: 1.0 } Children [ {EntityScene(rich_text(label, false))} ] ),
            ( Node { column_gap: Val::Px({spacing::M}), align_items: AlignItems::Center } Children [ {chips} ] ),
        ]
    }
}

fn pick_on_click(
    click: On<Pointer<Click>>,
    rows: Query<&ChoiceRow>,
    parents: Query<&ChildOf>,
    lists: Query<&ChoiceList>,
    is_list: Query<(), With<ChoiceList>>,
    gestures: CatalogInput,
    mut commands: Commands,
) {
    let Ok(row) = rows.get(click.entity) else {
        return;
    };
    let Some(list) = ancestor_with::<ChoiceList>(click.entity, &parents, &is_list) else {
        return;
    };
    if !lists
        .get(list)
        .is_ok_and(|choices| gestures.clicked(choices.pick, &click))
    {
        return;
    }
    let index = row.index;
    commands.queue(move |world: &mut World| pick_choice_at(world, list, index));
}

fn select_on_hover(
    over: On<Pointer<Over>>,
    rows: Query<&ChoiceRow>,
    parents: Query<&ChildOf>,
    is_list: Query<(), With<ChoiceList>>,
    mut lists: Query<&mut ChoiceList>,
) {
    let Ok(row) = rows.get(over.entity) else {
        return;
    };
    if let Some(list) = ancestor_with::<ChoiceList>(over.entity, &parents, &is_list)
        && let Ok(mut list) = lists.get_mut(list)
        && list.selected != row.index
    {
        list.selected = row.index;
    }
}

fn edge(ink: Color) -> Color {
    ink.with_alpha(0.55)
}

fn rows(world: &World, list: Entity) -> Vec<Entity> {
    world
        .get::<Children>(list)
        .map(|kids| {
            kids.iter()
                .filter(|kid| world.get::<ChoiceRow>(*kid).is_some())
                .collect()
        })
        .unwrap_or_default()
}

fn row_count(world: &World, list: Entity) -> usize {
    rows(world, list).len()
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
