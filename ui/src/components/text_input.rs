use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_input::ButtonInput;
use bevy_input::keyboard::{Key, KeyCode, KeyboardInput};
use bevy_input_focus::{FocusedInput, InputFocus};
use bevy_scene::{Scene, bsn, on, template_value};
use bevy_text::{EditableText, TextEdit};
use bevy_ui::{UiRect, Val};

use crate::component;
use crate::components::text::font;
use crate::style::Style;
use crate::theme::theme;
use crate::tokens::typography;

type Submit = Arc<dyn Fn(&mut World, String) + Send + Sync>;

#[derive(Component, Clone)]
pub struct OnSubmit(pub Submit);

impl OnSubmit {
    pub fn new(handler: impl Fn(&mut World, String) + Send + Sync + 'static) -> OnSubmit {
        OnSubmit(Arc::new(handler))
    }
}

pub struct TextInputOptions {
    pub on_submit: OnSubmit,
}

/// Single-line text input; Enter submits the trimmed text and clears the field.
pub fn text_input(opts: TextInputOptions) -> impl Scene {
    let family = theme().surface_inset;
    bsn! {
        component(EditableText::default())
        component(opts.on_submit)
        component(font(typography::BODY))
        template_value(Style::new()
            .background(family.base)
            .text_color(family.on)
            .border_color(family.border)
            .node(|node| {
                node.width = Val::Percent(100.0);
                node.padding = UiRect::axes(Val::Px(6.0), Val::Px(3.0));
                node.border = UiRect::all(Val::Px(1.0));
            }))
        on(submit_on_enter)
    }
}

pub fn typing(focus: Option<Res<InputFocus>>, fields: Query<(), With<EditableText>>) -> bool {
    focus
        .and_then(|focus| focus.get())
        .is_some_and(|entity| fields.contains(entity))
}

pub(crate) fn blur_on_escape(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    fields: Query<(), With<EditableText>>,
    focus: Option<ResMut<InputFocus>>,
) {
    let (Some(keys), Some(mut focus)) = (keys, focus) else {
        return;
    };
    if keys.just_pressed(KeyCode::Escape)
        && focus.get().is_some_and(|entity| fields.contains(entity))
    {
        focus.clear();
    }
}

#[derive(Component)]
pub(crate) struct SubmitRequested;

fn submit_on_enter(
    mut input: On<FocusedInput<KeyboardInput>>,
    fields: Query<&EditableText, With<OnSubmit>>,
    mut commands: Commands,
) {
    if input.input.logical_key != Key::Enter || !input.input.state.is_pressed() {
        return;
    }
    let Ok(field) = fields.get(input.focused_entity) else {
        return;
    };
    if field.is_composing() {
        return;
    }
    input.propagate(false);
    commands
        .entity(input.focused_entity)
        .insert(SubmitRequested);
}

/// Submits only once every edit queued before (or alongside) the Enter press has been applied,
/// so the submitted text is never missing a same-frame keystroke or in-flight paste.
pub(crate) fn apply_submits(
    mut fields: Query<(Entity, &mut EditableText, &OnSubmit), With<SubmitRequested>>,
    mut commands: Commands,
) {
    for (entity, mut field, submit) in &mut fields {
        if !field.pending_edits.is_empty() || field.pending_paste.is_some() {
            continue;
        }
        commands.entity(entity).remove::<SubmitRequested>();
        let text = field.value().to_string().trim().to_owned();
        field.queue_edit(TextEdit::SelectAll);
        field.queue_edit(TextEdit::Backspace);
        if text.is_empty() {
            continue;
        }
        let submit = submit.0.clone();
        commands.queue(move |world: &mut World| submit(world, text));
    }
}
