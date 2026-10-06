use std::borrow::Cow;
use std::time::Duration;

use bevy_camera::visibility::Visibility;
use bevy_color::{Alpha, Color};
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::prelude::Pickable;
use bevy_scene::{CommandsSceneExt, Scene, bsn, template_value};
use bevy_text::{FontStyle, FontWeight, TextColor, TextFont, TextSpan};
use bevy_time::Time;
use bevy_ui::widget::Text;
use bevy_ui::{AlignItems, FlexDirection, FlexWrap, Node, UiRect, UiTransform, Val, Val2};

use crate::component;
use crate::components::input::{InputCatalog, InputRef, input_cap};
use crate::components::text::font;
use crate::theme::theme;
use crate::tokens::typography;

const SLOW_PACE: f32 = 2.5;
const WHISPER_SCALE: f32 = 0.9;
const WHISPER_ALPHA: f32 = 0.7;
const SHOUT_SCALE: f32 = 1.3;
const SHOUT_WEIGHT: u16 = 700;
const WORD_GAP_EM: f32 = 0.27;

#[derive(Clone, Debug, PartialEq)]
pub enum RichPiece {
    Span(RichSpan),
    Pause(Duration),
    Input(InputRef),
}

impl RichPiece {
    pub fn text(text: impl Into<Cow<'static, str>>) -> RichPiece {
        RichPiece::Span(RichSpan::plain(text))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RichSpan {
    pub text: Cow<'static, str>,
    pub color: Option<Color>,
    pub voice: TextVoice,
    pub motion: Option<TextMotion>,
    pub slow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextVoice {
    #[default]
    Normal,
    Whisper,
    Shout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextMotion {
    Wave,
    Shake,
    Pulse,
}

impl RichSpan {
    pub fn plain(text: impl Into<Cow<'static, str>>) -> RichSpan {
        RichSpan {
            text: text.into(),
            color: None,
            voice: TextVoice::Normal,
            motion: None,
            slow: false,
        }
    }

    pub fn color(mut self, color: Color) -> RichSpan {
        self.color = Some(color);
        self
    }

    pub fn voice(mut self, voice: TextVoice) -> RichSpan {
        self.voice = voice;
        self
    }

    pub fn motion(mut self, motion: TextMotion) -> RichSpan {
        self.motion = Some(motion);
        self
    }

    pub fn slow(mut self) -> RichSpan {
        self.slow = true;
        self
    }
}

impl From<RichSpan> for RichPiece {
    fn from(span: RichSpan) -> RichPiece {
        RichPiece::Span(span)
    }
}

#[derive(Component, Clone, Debug, PartialEq)]
#[require(Node)]
pub struct RichText {
    pub pieces: Vec<RichPiece>,
    pub size: f32,
    pub color: Color,
}

impl RichText {
    pub fn new(pieces: Vec<RichPiece>) -> RichText {
        RichText {
            pieces,
            size: typography::BODY.font_size,
            color: theme().surface_canvas.on,
        }
    }

    pub fn plain(&self, catalog: &InputCatalog) -> String {
        self.pieces
            .iter()
            .filter_map(|piece| match piece {
                RichPiece::Span(span) => Some(span.text.as_ref()),
                RichPiece::Pause(_) => None,
                RichPiece::Input(input) => catalog.name(*input),
            })
            .collect()
    }
}

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TypewriterSpeed(pub Option<f32>);

impl Default for TypewriterSpeed {
    fn default() -> TypewriterSpeed {
        TypewriterSpeed(Some(45.0))
    }
}

#[derive(Resource, Clone, Copy, Debug, PartialEq, Default)]
pub struct MotionPreference {
    pub reduced: bool,
}

#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Typewriter {
    animated: bool,
    elapsed: Duration,
    finished: bool,
    shown: usize,
}

#[derive(Message, Clone, Copy, Debug)]
pub struct TypewriterReveal {
    pub entity: Entity,
    pub index: usize,
}

impl Typewriter {
    pub fn finish(&mut self) {
        self.finished = true;
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

pub fn rich_text(text: RichText, typed: bool) -> impl Scene {
    let gap = text.size * WORD_GAP_EM;
    let node = Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        align_items: AlignItems::Baseline,
        column_gap: Val::Px(gap),
        margin: UiRect::right(Val::Px(-gap)),
        ..Default::default()
    };
    let typewriter = Typewriter {
        animated: typed,
        elapsed: Duration::ZERO,
        finished: !typed,
        shown: 0,
    };
    bsn! {
        template_value(node)
        component(text)
        component(typewriter)
        Pickable { should_block_lower: false, is_hoverable: false }
    }
}

pub fn reveal_times(pieces: &[RichPiece], speed: TypewriterSpeed) -> Vec<Duration> {
    let Some(per_second) = speed.0.filter(|rate| *rate > 0.0) else {
        return pieces
            .iter()
            .flat_map(|piece| match piece {
                RichPiece::Span(span) => vec![Duration::ZERO; span.text.chars().count()],
                RichPiece::Pause(_) => Vec::new(),
                RichPiece::Input(_) => vec![Duration::ZERO],
            })
            .collect();
    };
    let step = |pace: f32| {
        Duration::from_nanos((f64::from(pace) / f64::from(per_second) * 1e9).round() as u64)
    };
    let mut clock = Duration::ZERO;
    let mut times = Vec::new();
    for piece in pieces {
        match piece {
            RichPiece::Pause(pause) => clock += *pause,
            RichPiece::Span(span) => {
                let pace = if span.slow { SLOW_PACE } else { 1.0 };
                for _ in span.text.chars() {
                    clock += step(pace);
                    times.push(clock);
                }
            }
            RichPiece::Input(_) => {
                clock += step(1.0);
                times.push(clock);
            }
        }
    }
    times
}

pub fn revealed(pieces: &[RichPiece], speed: TypewriterSpeed, elapsed: Duration) -> usize {
    reveal_times(pieces, speed)
        .iter()
        .take_while(|time| **time <= elapsed)
        .count()
}

pub fn reveal_duration(pieces: &[RichPiece], speed: TypewriterSpeed) -> Duration {
    reveal_times(pieces, speed)
        .last()
        .copied()
        .unwrap_or(Duration::ZERO)
}

#[derive(Component)]
pub(crate) struct Word {
    index: usize,
    motion: Option<TextMotion>,
    segments: Vec<Segment>,
    cap_at: Option<usize>,
}

struct Segment {
    text: String,
    first_char: usize,
    visible: Entity,
    hidden: Entity,
}

#[derive(Component)]
pub(crate) struct Laid(Vec<Entity>);

pub(crate) fn lay_out_rich_text(
    mut commands: Commands,
    texts: Query<(Entity, &RichText, Option<&Laid>), Changed<RichText>>,
    mut typewriters: Query<&mut Typewriter>,
) {
    for (entity, text, laid) in &texts {
        if let Some(laid) = laid {
            for word in &laid.0 {
                commands.entity(*word).despawn();
            }
        }
        if let Ok(mut typewriter) = typewriters.get_mut(entity) {
            typewriter.elapsed = Duration::ZERO;
            typewriter.finished = !typewriter.animated;
            typewriter.shown = 0;
        }
        let mut laid: Vec<Entity> = split_words(text)
            .into_iter()
            .enumerate()
            .map(|(index, word)| spawn_word(&mut commands, entity, text, index, word))
            .collect();
        // Taffy breaks a wrapping row by comparing float sums exactly, so a row sized to its own
        // content can push its last word onto a new line. This empty trailing item, cancelled out
        // by the root's negative right margin, gives the last word a gap's worth of headroom.
        laid.push(commands.spawn((Node::default(), ChildOf(entity))).id());
        commands.entity(entity).insert(Laid(laid));
    }
}

pub(crate) fn type_rich_text(
    time: Res<Time>,
    speed: Res<TypewriterSpeed>,
    mut texts: Query<(Entity, &RichText, &mut Typewriter, Option<&Laid>)>,
    mut words: Query<(&Word, &mut Visibility)>,
    mut spans: Query<&mut TextSpan>,
    mut reveals: MessageWriter<TypewriterReveal>,
) {
    for (entity, text, mut typewriter, laid) in &mut texts {
        let Some(laid) = laid else {
            continue;
        };
        let shown = if typewriter.finished {
            usize::MAX
        } else {
            typewriter.elapsed += time.delta();
            let shown = revealed(&text.pieces, *speed, typewriter.elapsed);
            if typewriter.elapsed >= reveal_duration(&text.pieces, *speed) {
                typewriter.finished = true;
            }
            shown
        };
        if typewriter.animated && shown > typewriter.shown {
            let letters = reveal_times(&text.pieces, *speed).len();
            let newly = typewriter.shown..shown.min(letters);
            reveals.write_batch(newly.map(|index| TypewriterReveal { entity, index }));
            typewriter.shown = shown;
        }
        let mut laid_words = words.iter_many_mut(&laid.0);
        while let Some((word, mut visibility)) = laid_words.fetch_next() {
            if let Some(at) = word.cap_at {
                let next = if shown > at {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                if *visibility != next {
                    *visibility = next;
                }
            }
            for segment in &word.segments {
                let visible = shown.saturating_sub(segment.first_char);
                let split = segment
                    .text
                    .char_indices()
                    .nth(visible)
                    .map_or(segment.text.len(), |(at, _)| at);
                let (head, tail) = segment.text.split_at(split);
                if let Ok(mut span) = spans.get_mut(segment.visible)
                    && span.0 != head
                {
                    span.0 = head.to_owned();
                }
                if let Ok(mut span) = spans.get_mut(segment.hidden)
                    && span.0 != tail
                {
                    span.0 = tail.to_owned();
                }
            }
        }
    }
}

pub(crate) fn move_words(
    time: Res<Time>,
    preference: Res<MotionPreference>,
    mut words: Query<(&Word, &mut UiTransform)>,
) {
    let t = time.elapsed_secs();
    for (word, mut transform) in &mut words {
        let phase = word.index as f32;
        let (offset, scale) = match word.motion.filter(|_| !preference.reduced) {
            None => (Vec2::ZERO, 1.0),
            Some(TextMotion::Wave) => (Vec2::new(0.0, (t * 5.0 - phase * 0.8).sin() * 2.5), 1.0),
            Some(TextMotion::Shake) => (
                Vec2::new(
                    (t * 53.0 + phase * 7.1).sin() * 1.2,
                    (t * 47.0 + phase * 3.3).cos() * 1.2,
                ),
                1.0,
            ),
            Some(TextMotion::Pulse) => (Vec2::ZERO, 1.0 + (t * 3.0 - phase * 0.4).sin() * 0.05),
        };
        let next = UiTransform {
            translation: Val2::px(offset.x, offset.y),
            scale: Vec2::splat(scale),
            ..*transform
        };
        if *transform != next {
            *transform = next;
        }
    }
}

struct WordPlan {
    motion: Option<TextMotion>,
    parts: Vec<(usize, String, usize)>,
    cap: Option<(InputRef, usize)>,
}

fn split_words(text: &RichText) -> Vec<WordPlan> {
    let mut words: Vec<WordPlan> = Vec::new();
    let mut open = false;
    let mut char_index = 0;
    for (span_index, piece) in text.pieces.iter().enumerate() {
        let span = match piece {
            RichPiece::Span(span) => span,
            RichPiece::Pause(_) => continue,
            RichPiece::Input(input) => {
                words.push(WordPlan {
                    motion: None,
                    parts: Vec::new(),
                    cap: Some((*input, char_index)),
                });
                open = false;
                char_index += 1;
                continue;
            }
        };
        for ch in span.text.chars() {
            if ch.is_whitespace() {
                open = false;
                char_index += 1;
                continue;
            }
            if !open {
                words.push(WordPlan {
                    motion: span.motion,
                    parts: Vec::new(),
                    cap: None,
                });
                open = true;
            }
            let word = words.last_mut().expect("a word was just opened");
            word.motion = word.motion.or(span.motion);
            match word.parts.last_mut() {
                Some((index, part, _)) if *index == span_index => part.push(ch),
                _ => word.parts.push((span_index, ch.to_string(), char_index)),
            }
            char_index += 1;
        }
    }
    words
}

fn spawn_word(
    commands: &mut Commands,
    parent: Entity,
    text: &RichText,
    index: usize,
    plan: WordPlan,
) -> Entity {
    if let Some((input, at)) = plan.cap {
        return commands
            .spawn_scene(input_cap(input, text.size, text.color))
            .insert((
                Word {
                    index,
                    motion: None,
                    segments: Vec::new(),
                    cap_at: Some(at),
                },
                UiTransform::default(),
                ChildOf(parent),
            ))
            .id();
    }
    let word = commands
        .spawn((
            Node::default(),
            Text::new(""),
            TextFont {
                font_size: text.size.into(),
                ..font(typography::BODY)
            },
            UiTransform::default(),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id();
    let segments = plan
        .parts
        .into_iter()
        .map(|(span_index, part, first_char)| {
            let RichPiece::Span(span) = &text.pieces[span_index] else {
                unreachable!("words are only built from spans");
            };
            let (style, color) = span_style(text, span);
            let visible = commands
                .spawn((
                    TextSpan::new(String::new()),
                    style.clone(),
                    TextColor(color),
                    Pickable::IGNORE,
                    ChildOf(word),
                ))
                .id();
            let hidden = commands
                .spawn((
                    TextSpan::new(part.clone()),
                    style,
                    TextColor(color.with_alpha(0.0)),
                    Pickable::IGNORE,
                    ChildOf(word),
                ))
                .id();
            Segment {
                text: part,
                first_char,
                visible,
                hidden,
            }
        })
        .collect();
    commands.entity(word).insert(Word {
        index,
        motion: plan.motion,
        segments,
        cap_at: None,
    });
    word
}

fn span_style(text: &RichText, span: &RichSpan) -> (TextFont, Color) {
    let base = font(typography::BODY);
    let color = span.color.unwrap_or(text.color);
    match span.voice {
        TextVoice::Normal => (
            TextFont {
                font_size: text.size.into(),
                ..base
            },
            color,
        ),
        TextVoice::Whisper => (
            TextFont {
                font_size: (text.size * WHISPER_SCALE).into(),
                style: FontStyle::Italic,
                ..base
            },
            color.with_alpha(color.alpha() * WHISPER_ALPHA),
        ),
        TextVoice::Shout => (
            TextFont {
                font_size: (text.size * SHOUT_SCALE).into(),
                weight: FontWeight(SHOUT_WEIGHT),
                ..base
            },
            color,
        ),
    }
}
