use std::ops::Range;
use std::sync::Arc;
use ui::egui;

use bevy::prelude::World;
use ui::egui::emath::TSTransform;
use ui::egui::epaint::TextShape;
use ui::egui::text::{LayoutJob, TextFormat};
use ui::egui::{
    Color32, FontId, Galley, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
    WidgetInfo, WidgetType,
};
use ui::face::{Face, TextRole};
use ui::ink::Ground;

use super::{KeyNames, Lettering, SpanInk, StyledLine, Swing, TextMotion, TextPiece, TextVoice};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextLook {
    pub size: f32,
    pub color: Color32,
    pub hand: Hand,
    pub line_height: Option<f32>,
    pub ground: Ground,
    /// A soft ring of this color around every letter, for text set straight over the world.
    pub halo: Option<Color32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Book,
    /// The book face set heavy, for a line that stands over the world rather than on a page.
    Bold,
    Quill,
}

impl TextLook {
    pub const fn new(size: f32, color: Color32) -> TextLook {
        TextLook {
            size,
            color,
            hand: Hand::Book,
            line_height: None,
            ground: Ground::Paper,
            halo: None,
        }
    }

    pub fn role(role: TextRole, color: Color32) -> TextLook {
        TextLook {
            line_height: role.line_height(),
            ..TextLook::new(role.size(), color)
        }
    }

    pub const fn on(self, ground: Ground) -> TextLook {
        TextLook { ground, ..self }
    }

    pub const fn haloed(self, halo: Color32) -> TextLook {
        TextLook {
            halo: Some(halo),
            ..self
        }
    }

    pub const fn bold(self) -> TextLook {
        TextLook {
            hand: Hand::Bold,
            ..self
        }
    }

    pub const fn quill(self) -> TextLook {
        TextLook {
            hand: Hand::Quill,
            ..self
        }
    }
}

/// The line is laid out whole, so revealing letters never reflows it.
pub fn show(
    ui: &mut Ui,
    world: &World,
    line: &StyledLine,
    look: TextLook,
    shown: Option<usize>,
) -> Response {
    let keys = world.resource::<KeyNames>();
    let plan = Plan::new(line, keys, *world.resource::<Lettering>(), look);
    let galley = ui.painter().layout_job(plan.job(ui.available_width()));
    let pad = plan.pad;
    let (rect, response) = ui.allocate_exact_size(plan.footprint(&galley), Sense::hover());
    let plain = line.plain(keys);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &plain));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let origin = rect.min - galley.rect.min.to_vec2();
    let placed = place(&galley, &plan, origin);
    let shown = shown.unwrap_or(plan.units);
    let time = ui.input(|input| input.time) as f32;
    for cap in plan.caps.iter().filter(|cap| cap.letter < shown) {
        paint_cap(ui, &placed, cap, pad);
    }
    for (index, word) in plan.words.iter().enumerate() {
        let visible = word.letters.start
            ..word
                .letters
                .clone()
                .take_while(|at| plan.letters[*at].letter < shown)
                .last()
                .map_or(word.letters.start, |last| last + 1);
        if visible.is_empty() {
            continue;
        }
        let Some(at) = placed[word.letters.start] else {
            continue;
        };
        paint_word(ui, &plan, visible, at, word.motion, index as f32, time);
    }
    response
}

pub fn measure(
    gui: &egui::Context,
    world: &World,
    line: &StyledLine,
    look: TextLook,
    wrap: f32,
) -> Vec2 {
    let plan = Plan::new(
        line,
        world.resource::<KeyNames>(),
        *world.resource::<Lettering>(),
        look,
    );
    let galley = gui.fonts_mut(|fonts| fonts.layout_job(plan.job(wrap)));
    plan.footprint(&galley)
}

const HALO_STEPS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadow {
    pub offset: Vec2,
    pub color: Color32,
}

/// `shift` moves and scales the drawing about the text's centre while the space it takes stays put,
/// so a moving title never reflows what is around it.
pub fn show_shadowed(
    ui: &mut Ui,
    text: &str,
    font: FontId,
    color: Color32,
    shadow: TextShadow,
    shift: TSTransform,
) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let centre = rect.center().to_vec2();
    let around =
        TSTransform::from_translation(centre) * shift * TSTransform::from_translation(-centre);
    let mut under = TextShape::new(rect.min + shadow.offset, galley.clone(), shadow.color);
    under.override_text_color = Some(shadow.color);
    under.transform(around);
    let mut over = TextShape::new(rect.min, galley, color);
    over.transform(around);
    ui.painter().add(Shape::Text(under));
    ui.painter().add(Shape::Text(over));
    response
}

struct Plan {
    letters: Vec<Letter>,
    words: Vec<Word>,
    caps: Vec<Cap>,
    units: usize,
    pad: f32,
    halo: Option<Color32>,
    lettering: Lettering,
}

struct Letter {
    ch: char,
    format: TextFormat,
    leading: f32,
    letter: usize,
}

struct Word {
    letters: Range<usize>,
    motion: Option<TextMotion>,
}

struct Cap {
    letters: Range<usize>,
    letter: usize,
    color: Color32,
}

#[derive(Clone, Copy)]
struct Placed {
    baseline: Pos2,
    advance: f32,
    ascent: f32,
    height: f32,
}

impl Plan {
    fn new(line: &StyledLine, keys: &KeyNames, lettering: Lettering, look: TextLook) -> Plan {
        let pad = look.size * lettering.caps.room;
        let mut plan = Plan {
            letters: Vec::new(),
            words: Vec::new(),
            caps: Vec::new(),
            units: 0,
            pad,
            halo: look.halo,
            lettering,
        };
        let mut letter = 0;
        let mut owed = 0.0;
        let mut open = false;
        for piece in &line.pieces {
            match piece {
                TextPiece::Pause(_) => {}
                TextPiece::Key(cap) => {
                    let format = TextFormat {
                        line_height: look.line_height,
                        ..TextFormat::simple(
                            Face::Mono.id(look.size * lettering.caps.size),
                            look.color,
                        )
                    };
                    let start = plan.letters.len();
                    for (at, ch) in cap.name(keys).chars().enumerate() {
                        let leading = if at == 0 { owed + pad } else { 0.0 };
                        plan.letters.push(Letter {
                            ch,
                            format: format.clone(),
                            leading,
                            letter,
                        });
                    }
                    let letters = start..plan.letters.len();
                    plan.words.push(Word {
                        letters: letters.clone(),
                        motion: None,
                    });
                    plan.caps.push(Cap {
                        letters,
                        letter,
                        color: look.color,
                    });
                    owed = pad;
                    open = false;
                    letter += 1;
                }
                TextPiece::Span(span) => {
                    let format = span_format(span.voice, ink_of(span.ink, look), look, lettering);
                    for ch in span.text.chars() {
                        let leading = std::mem::take(&mut owed);
                        plan.letters.push(Letter {
                            ch,
                            format: format.clone(),
                            leading,
                            letter,
                        });
                        letter += 1;
                        let at = plan.letters.len() - 1;
                        if ch.is_whitespace() {
                            open = false;
                            continue;
                        }
                        match plan.words.last_mut() {
                            Some(word) if open => {
                                word.letters.end = at + 1;
                                word.motion = word.motion.or(span.motion);
                            }
                            _ => plan.words.push(Word {
                                letters: at..at + 1,
                                motion: span.motion,
                            }),
                        }
                        open = true;
                    }
                }
            }
        }
        plan.units = letter;
        plan
    }

    fn footprint(&self, galley: &Galley) -> Vec2 {
        let pad = if self.caps.is_empty() { 0.0 } else { self.pad };
        galley.size() + Vec2::new(pad, 0.0)
    }

    fn job(&self, wrap: f32) -> LayoutJob {
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap;
        for letter in &self.letters {
            let mut buffer = [0; 4];
            job.append(
                letter.ch.encode_utf8(&mut buffer),
                letter.leading,
                letter.format.clone(),
            );
        }
        job
    }
}

fn span_format(
    voice: Option<TextVoice>,
    color: Color32,
    look: TextLook,
    lettering: Lettering,
) -> TextFormat {
    let (normal, whisper, shout) = match look.hand {
        Hand::Book => (Face::Book, Face::BookItalic, Face::BookBold),
        Hand::Bold => (Face::BookBold, Face::BookItalic, Face::BookBold),
        Hand::Quill => (Face::Quill, Face::Quill, Face::Quill),
    };
    let format = match voice {
        None => TextFormat::simple(normal.id(look.size), color),
        Some(TextVoice::Whisper) => TextFormat::simple(
            whisper.id(look.size * lettering.whisper.size),
            color.gamma_multiply(lettering.whisper.opacity),
        ),
        Some(TextVoice::Shout) => {
            TextFormat::simple(shout.id(look.size * lettering.shout_size), color)
        }
    };
    TextFormat {
        line_height: look.line_height,
        ..format
    }
}

/// egui keeps one glyph per character in its rows, leaving out only the newlines that end them.
fn place(galley: &Galley, plan: &Plan, origin: Pos2) -> Vec<Option<Placed>> {
    let mut glyphs = galley.rows.iter().flat_map(|placed| {
        placed.row.glyphs.iter().map(move |glyph| Placed {
            baseline: origin + placed.pos.to_vec2() + glyph.pos.to_vec2(),
            advance: glyph.advance_width,
            ascent: glyph.font_ascent,
            height: glyph.font_height,
        })
    });
    plan.letters
        .iter()
        .map(|letter| {
            if letter.ch == '\n' {
                None
            } else {
                glyphs.next()
            }
        })
        .collect()
}

fn paint_cap(ui: &Ui, placed: &[Option<Placed>], cap: &Cap, pad: f32) {
    let glyphs: Vec<Placed> = cap.letters.clone().filter_map(|at| placed[at]).collect();
    let (Some(first), Some(last)) = (glyphs.first(), glyphs.last()) else {
        return;
    };
    let top = first.baseline.y - first.ascent;
    let rect = Rect::from_min_max(
        Pos2::new(first.baseline.x - pad, top),
        Pos2::new(last.baseline.x + last.advance + pad, top + first.height),
    );
    ui.painter().rect(
        rect,
        3.0,
        cap.color.gamma_multiply(0.12),
        Stroke::new(1.0, cap.color.gamma_multiply(0.5)),
        StrokeKind::Inside,
    );
}

fn paint_word(
    ui: &Ui,
    plan: &Plan,
    letters: Range<usize>,
    at: Placed,
    motion: Option<TextMotion>,
    phase: f32,
    time: f32,
) {
    let mut job = LayoutJob::default();
    for letter in &plan.letters[letters] {
        let mut buffer = [0; 4];
        job.append(
            letter.ch.encode_utf8(&mut buffer),
            0.0,
            letter.format.clone(),
        );
    }
    let galley: Arc<Galley> = ui.painter().layout_job(job);
    let Some(first) = galley
        .rows
        .first()
        .and_then(|row| Some(row.pos + row.row.glyphs.first()?.pos.to_vec2()))
    else {
        return;
    };
    let pos = at.baseline - first.to_vec2();
    let mut shape = TextShape::new(pos, galley, Color32::PLACEHOLDER);
    match motion {
        None => {}
        Some(TextMotion::Wave) => {
            let wave = plan.lettering.wave;
            shape.pos.y += turn(wave, time, -phase).sin() * wave.reach;
        }
        Some(TextMotion::Shake) => {
            let shake = plan.lettering.shake;
            shape.pos += Vec2::new(
                turn(shake.across, time, phase).sin() * shake.across.reach,
                turn(shake.down, time, phase).cos() * shake.down.reach,
            );
        }
        Some(TextMotion::Pulse) => {
            let pulse = plan.lettering.pulse;
            let scale = 1.0 + turn(pulse, time, -phase).sin() * pulse.reach;
            let center = shape.visual_bounding_rect().center().to_vec2();
            shape.transform(TSTransform::new(center * (1.0 - scale), scale));
        }
    }
    if let Some(halo) = plan.halo {
        for step in 0..HALO_STEPS {
            let angle = step as f32 / HALO_STEPS as f32 * std::f32::consts::TAU;
            let mut ring = shape.clone();
            ring.pos += Vec2::angled(angle) * plan.lettering.halo;
            ring.override_text_color = Some(halo);
            ui.painter().add(Shape::Text(ring));
        }
    }
    ui.painter().add(Shape::Text(shape));
}

fn ink_of(ink: Option<SpanInk>, look: TextLook) -> Color32 {
    match ink {
        Some(SpanInk::Prose(prose)) => prose.on(look.ground),
        Some(SpanInk::Fixed(color)) => color,
        None => look.color,
    }
}

/// How far round its swing a word `phase` words into its line has turned at `time`.
fn turn(swing: Swing, time: f32, phase: f32) -> f32 {
    time * swing.rate.0 + phase * swing.step.0
}
