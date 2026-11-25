use std::ops::Range;
use std::sync::Arc;
use ui::egui;

use bevy::prelude::World;
use ui::egui::emath::TSTransform;
use ui::egui::epaint::TextShape;
use ui::egui::text::{LayoutJob, TextFormat};
use ui::egui::{
    Align, Color32, FontFamily, FontId, Galley, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, WidgetInfo, WidgetType,
};
use ui::face::{Face, TextRole};
use ui::ink::{self, Ground};
use ui::paint::TextShadow;

use super::{
    KeyNames, Lettering, LinkKey, SpanInk, StyledLine, Swing, TextMotion, TextPiece, TextVoice,
};
use crate::core::gui;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextLook {
    pub size: f32,
    pub color: Color32,
    pub hand: Hand,
    pub line_height: Option<f32>,
    pub ground: Ground,
    /// A soft ring of this color around every letter, for text set straight over the world.
    pub halo: Option<Color32>,
    /// Copies of every letter drawn under it, for text set over the world.
    pub shadows: &'static [TextShadow],
    /// How far the shadows reach, as a share of how far they are set to.
    pub shadow_reach: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Book,
    /// The book face set heavy, for a line that stands over the world rather than on a page.
    Bold,
    /// The book face set a little heavier, for outlined text, whose outline would thin the
    /// strokes of the regular weight.
    Medium,
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
            shadows: &[],
            shadow_reach: 1.0,
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

    pub const fn shadowed(self, shadows: &'static [TextShadow]) -> TextLook {
        TextLook { shadows, ..self }
    }

    /// The shadows reaching `reach` of how far they are set to, for text of another size than
    /// they were set for.
    pub const fn shadows_reaching(self, reach: f32) -> TextLook {
        TextLook {
            shadow_reach: reach,
            ..self
        }
    }

    pub const fn bold(self) -> TextLook {
        TextLook {
            hand: Hand::Bold,
            ..self
        }
    }

    pub const fn medium(self) -> TextLook {
        TextLook {
            hand: Hand::Medium,
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
    show_linked(ui, world, line, look, shown).response
}

/// A line as drawn, and where each run of it that stands for a link landed: one rect a row it
/// spans, those not yet shown left out.
pub struct ShownLine {
    pub response: Response,
    pub links: Vec<ShownLink>,
    /// Where each row of the line's letters stands, typed out yet or not.
    pub rows: Vec<Rect>,
}

/// A row of a link's run as drawn, with the name the run is read out by.
pub struct ShownLink {
    pub key: LinkKey,
    pub name: String,
    pub row: Rect,
}

/// Draws `line` as `show` does, and tells where its links landed.
pub fn show_linked(
    ui: &mut Ui,
    world: &World,
    line: &StyledLine,
    look: TextLook,
    shown: Option<usize>,
) -> ShownLine {
    let keys = world.resource::<KeyNames>();
    let plan = Plan::new(line, keys, *world.resource::<Lettering>(), look);
    let galley = ui.painter().layout_job(plan.job(ui.available_width()));
    let pad = plan.pad;
    let (rect, response) = ui.allocate_exact_size(plan.footprint(&galley), Sense::hover());
    let plain = line.plain(keys);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &plain));
    let origin = rect.min - galley.rect.min.to_vec2();
    let rows = galley
        .rows
        .iter()
        .map(|placed| placed.rect().translate(origin.to_vec2()))
        .collect();
    if !ui.is_rect_visible(rect) {
        return ShownLine {
            response,
            links: Vec::new(),
            rows,
        };
    }
    let placed = place(&galley, &plan, origin);
    let shown = shown.unwrap_or(plan.units);
    let time = ui.input(|input| input.time) as f32;
    for cap in plan.caps.iter().filter(|cap| out(cap.typing, shown)) {
        paint_cap(ui, &placed, cap, pad, plan.over_world);
    }
    for image in plan.images.iter().filter(|image| out(image.typing, shown)) {
        paint_image(ui, world, &placed, image);
    }
    for (index, word) in plan.words.iter().enumerate() {
        for run in visible_runs(&plan, word.letters.clone(), shown) {
            let Some(at) = placed[run.start] else {
                continue;
            };
            paint_word(ui, &plan, run, at, word.motion, index as f32, time);
        }
    }
    let links = plan
        .links
        .iter()
        .flat_map(|link| {
            link_rows(&plan, &placed, link.letters.clone(), shown)
                .into_iter()
                .map(|row| ShownLink {
                    key: link.key,
                    name: link.name.clone(),
                    row,
                })
        })
        .collect();
    ShownLine {
        response,
        links,
        rows,
    }
}

/// Whether a letter, cap or picture is out once `shown` of the line's typing units are: what
/// is not part of the typing is always out.
fn out(typing: Option<usize>, shown: usize) -> bool {
    typing.is_none_or(|unit| unit < shown)
}

/// The stretches of `letters` that are out, each laid out where the whole line put it.
fn visible_runs(plan: &Plan, letters: Range<usize>, shown: usize) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for at in letters {
        if !out(plan.letters[at].typing, shown) {
            continue;
        }
        match runs.last_mut() {
            Some(run) if run.end == at => run.end = at + 1,
            _ => runs.push(at..at + 1),
        }
    }
    runs
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
/// What a cap over the world stands on.
const CAP_GROUND: Color32 = Color32::from_black_alpha(170);

struct Plan {
    letters: Vec<Letter>,
    words: Vec<Word>,
    caps: Vec<Cap>,
    images: Vec<SetImage>,
    links: Vec<Link>,
    units: usize,
    pad: f32,
    halo: Option<Color32>,
    shadows: &'static [TextShadow],
    shadow_reach: f32,
    lettering: Lettering,
    /// Whether caps stand over the world, on a ground of their own, rather than on a page.
    over_world: bool,
}

struct Letter {
    ch: char,
    format: TextFormat,
    leading: f32,
    typing: Option<usize>,
    /// Whether the letter is one of the line's plain words, in the look's own size, whose baseline
    /// the line is set on.
    plain: bool,
    /// How much of the letter shows, its shadows with it.
    opacity: f32,
}

struct Word {
    letters: Range<usize>,
    motion: Option<TextMotion>,
}

struct Cap {
    letters: Range<usize>,
    typing: Option<usize>,
    color: Color32,
}

/// A run of the line's letters that stands for `key`.
struct Link {
    key: LinkKey,
    name: String,
    letters: Range<usize>,
}

/// A picture in the room left before the space at `at`.
struct SetImage {
    at: usize,
    typing: Option<usize>,
    file: String,
    side: f32,
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
            images: Vec::new(),
            links: Vec::new(),
            units: 0,
            pad,
            halo: look.halo,
            shadows: look.shadows,
            shadow_reach: look.shadow_reach,
            lettering,
            over_world: !look.shadows.is_empty() || look.halo.is_some(),
        };
        let marked = line
            .pieces
            .iter()
            .any(|piece| matches!(piece, TextPiece::TypingStart));
        let mut counted = !marked;
        let mut letter = 0;
        let mut owed = 0.0;
        let mut open = false;
        for piece in &line.pieces {
            match piece {
                TextPiece::Pause(_) => {}
                TextPiece::TypingStart => counted = true,
                TextPiece::TypingEnd => counted = false,
                TextPiece::Room(room) => owed += room,
                TextPiece::LinkStart { key, name } => {
                    let at = plan.letters.len();
                    plan.links.push(Link {
                        key: *key,
                        name: name.clone(),
                        letters: at..at,
                    });
                }
                TextPiece::LinkEnd => {
                    let end = plan.letters.len();
                    if let Some(link) = plan.links.last_mut() {
                        link.letters.end = end;
                    }
                }
                TextPiece::Image(file) => {
                    plan.images.push(SetImage {
                        at: plan.letters.len(),
                        typing: counted.then_some(letter),
                        file: file.to_string(),
                        side: look.size,
                    });
                    plan.letters.push(Letter {
                        ch: ' ',
                        format: span_format(None, false, look.color, look, lettering),
                        leading: std::mem::take(&mut owed) + look.size,
                        typing: counted.then_some(letter),
                        plain: false,
                        opacity: 1.0,
                    });
                    open = false;
                    if counted {
                        letter += 1;
                    }
                }
                TextPiece::Key(key) => {
                    let Some(cap) = key.cap(keys) else {
                        continue;
                    };
                    let size = look.size * lettering.caps.size;
                    let format = |scale: f32, family: FontFamily| TextFormat {
                        line_height: look.line_height,
                        valign: Align::Center,
                        ..TextFormat::simple(FontId::new(size * scale, family), look.color)
                    };
                    for (index, face) in cap.faces().enumerate() {
                        if index > 0 {
                            let at = plan.letters.len();
                            plan.letters.push(Letter {
                                ch: '+',
                                format: format(1.0, Face::Mono.family()),
                                leading: std::mem::take(&mut owed),
                                typing: counted.then_some(letter),
                                plain: false,
                                opacity: 1.0,
                            });
                            plan.words.push(Word {
                                letters: at..at + 1,
                                motion: None,
                            });
                        }
                        let start = plan.letters.len();
                        for run in face.runs() {
                            for ch in run.text.chars() {
                                let leading = if plan.letters.len() == start {
                                    std::mem::take(&mut owed) + pad
                                } else {
                                    0.0
                                };
                                plan.letters.push(Letter {
                                    ch,
                                    format: format(run.scale, run.family.clone()),
                                    leading,
                                    typing: counted.then_some(letter),
                                    plain: false,
                                    opacity: 1.0,
                                });
                            }
                        }
                        let letters = start..plan.letters.len();
                        plan.words.push(Word {
                            letters: letters.clone(),
                            motion: None,
                        });
                        plan.caps.push(Cap {
                            letters,
                            typing: counted.then_some(letter),
                            color: look.color,
                        });
                        owed = pad;
                    }
                    open = false;
                    if counted {
                        letter += 1;
                    }
                }
                TextPiece::Span(span) => {
                    let format = span_format(
                        span.voice,
                        span.bold,
                        ink_of(span.ink, look),
                        look,
                        lettering,
                    );
                    let opacity = match span.voice {
                        Some(TextVoice::Whisper) => lettering.whisper.opacity,
                        _ => 1.0,
                    };
                    for ch in span.text.chars() {
                        let leading = std::mem::take(&mut owed);
                        plan.letters.push(Letter {
                            ch,
                            format: format.clone(),
                            leading,
                            typing: counted.then_some(letter),
                            plain: span.voice.is_none(),
                            opacity,
                        });
                        if counted {
                            letter += 1;
                        }
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
    bold: bool,
    color: Color32,
    look: TextLook,
    lettering: Lettering,
) -> TextFormat {
    let (normal, whisper, shout) = match look.hand {
        Hand::Book => (Face::Book, Face::BookItalic, Face::BookBold),
        Hand::Bold => (Face::BookBold, Face::BookItalic, Face::BookBold),
        Hand::Medium => (Face::BookMedium, Face::BookMediumItalic, Face::BookBold),
        Hand::Quill => (Face::Quill, Face::Quill, Face::Quill),
    };
    let format = match voice {
        None if bold => TextFormat::simple(shout.id(look.size), color),
        None => TextFormat::simple(normal.id(look.size), color),
        Some(TextVoice::Whisper) => {
            TextFormat::simple(whisper.id(look.size * lettering.whisper.size), color)
        }
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
/// Every row is a line height tall, and egui hangs each glyph in it by its own font's ascent, so a
/// larger voice would sit below the rest of its row and a smaller one above. Every letter, cap and
/// picture is set instead on the baseline the line's plain words stand on in their row, which a
/// larger voice rises from.
fn place(galley: &Galley, plan: &Plan, origin: Pos2) -> Vec<Option<Placed>> {
    let mut glyphs = galley.rows.iter().flat_map(|placed| {
        let top = origin.y + placed.pos.y;
        placed.row.glyphs.iter().map(move |glyph| {
            let at = Placed {
                baseline: origin + placed.pos.to_vec2() + glyph.pos.to_vec2(),
                advance: glyph.advance_width,
                ascent: glyph.font_ascent,
                height: glyph.font_height,
            };
            (top, at)
        })
    });
    let placed: Vec<Option<(f32, Placed)>> = plan
        .letters
        .iter()
        .map(|letter| {
            if letter.ch == '\n' {
                None
            } else {
                glyphs.next()
            }
        })
        .collect();
    let drop = plan.letters.iter().zip(&placed).find_map(|(letter, at)| {
        let (top, at) = at.as_ref().filter(|_| letter.plain)?;
        Some(at.baseline.y - top)
    });
    placed
        .into_iter()
        .map(|at| {
            let (top, mut at) = at?;
            if let Some(drop) = drop {
                at.baseline.y = top + drop;
            }
            Some(at)
        })
        .collect()
}

/// Where the shown letters of `letters` landed, one rect for each row they run over: a picture's
/// space reaches back over the picture set before it.
fn link_rows(
    plan: &Plan,
    placed: &[Option<Placed>],
    letters: Range<usize>,
    shown: usize,
) -> Vec<Rect> {
    let mut rows: Vec<Rect> = Vec::new();
    for at in letters {
        let (Some(glyph), letter) = (placed[at], &plan.letters[at]) else {
            continue;
        };
        if !out(letter.typing, shown) {
            continue;
        }
        let picture = plan
            .images
            .iter()
            .find(|image| image.at == at)
            .map_or(0.0, |image| image.side);
        let top = glyph.baseline.y - glyph.ascent;
        let rect = Rect::from_min_max(
            Pos2::new(glyph.baseline.x - picture, top),
            Pos2::new(glyph.baseline.x + glyph.advance, top + glyph.height),
        );
        match rows.last_mut() {
            Some(row) if (row.top() - rect.top()).abs() < 1.0 => *row = row.union(rect),
            _ => rows.push(rect),
        }
    }
    rows
}

/// A cap on a page is a faint box in the text's own ink; one over the world stands on a dark
/// ground of its own, so it reads as a key over any scene.
fn paint_cap(ui: &Ui, placed: &[Option<Placed>], cap: &Cap, pad: f32, over_world: bool) {
    let glyphs: Vec<Placed> = cap.letters.clone().filter_map(|at| placed[at]).collect();
    let (Some(first), Some(last)) = (glyphs.first(), glyphs.last()) else {
        return;
    };
    let top = first.baseline.y - first.ascent;
    let rect = Rect::from_min_max(
        Pos2::new(first.baseline.x - pad, top),
        Pos2::new(last.baseline.x + last.advance + pad, top + first.height),
    );
    let (fill, stroke) = if over_world {
        (CAP_GROUND, cap.color.gamma_multiply(0.7))
    } else {
        (
            cap.color.gamma_multiply(0.12),
            cap.color.gamma_multiply(0.5),
        )
    };
    ui.painter().rect(
        rect,
        3.0,
        fill,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
}

fn paint_image(ui: &Ui, world: &World, placed: &[Option<Placed>], image: &SetImage) {
    let (Some(space), Some(picture)) = (placed[image.at], gui::kept_image(world, &image.file))
    else {
        return;
    };
    let middle = space.baseline.y - space.ascent + space.height / 2.0;
    ui.painter().image(
        picture.texture,
        Rect::from_min_size(
            Pos2::new(space.baseline.x - image.side, middle - image.side / 2.0),
            Vec2::splat(image.side),
        ),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
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
    let opacity = plan.letters[letters.start].opacity;
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
    for shadow in plan.shadows {
        let mut copy = shape.clone();
        copy.pos += shadow.offset * plan.shadow_reach;
        copy.override_text_color = Some(shadow.color.gamma_multiply(opacity));
        ui.painter().add(Shape::Text(copy));
    }
    if let Some(halo) = plan.halo {
        for step in 0..HALO_STEPS {
            let angle = step as f32 / HALO_STEPS as f32 * std::f32::consts::TAU;
            let mut ring = shape.clone();
            ring.pos += Vec2::angled(angle) * plan.lettering.halo;
            ring.override_text_color = Some(halo.gamma_multiply(opacity));
            ui.painter().add(Shape::Text(ring));
        }
    }
    shape.opacity_factor = opacity;
    ui.painter().add(Shape::Text(shape));
}

fn ink_of(ink: Option<SpanInk>, look: TextLook) -> Color32 {
    match ink {
        Some(SpanInk::Prose(prose)) => prose.on(look.ground),
        Some(SpanInk::Fixed(color)) => color,
        Some(SpanInk::Lit(color)) => match look.ground {
            Ground::Paper => look.color,
            Ground::Dusk => ink::legible(color, ink::VEIL),
        },
        Some(SpanInk::Legible(color)) => look.ground.legible(color),
        None => look.color,
    }
}

/// How far round its swing a word `phase` words into its line has turned at `time`.
fn turn(swing: Swing, time: f32, phase: f32) -> f32 {
    time * swing.rate.0 + phase * swing.step.0
}
