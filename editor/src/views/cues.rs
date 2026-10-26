//! A clip laid out as a strip of its moments: the playhead scrubbed and stepped, cues dragged,
//! marked at the playhead and removed. The model's overrides and the skeleton's clips both time
//! their cues on it.

use engine::core::time::Millis;
use engine::systems::actor::model::crossed_cues;
use engine::systems::actor::skeleton::Cue;
use ui::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use ui::glyph::Mark;
use ui::{form, theme};

use crate::keymap::EditorKey;

/// How far a step key moves the playhead.
pub const STEP: Millis = Millis(10.0);
/// How close to a cue, in points, the pointer takes it.
const GRIP: f32 = 8.0;
pub const STRIP_HEIGHT: f32 = 18.0;
const TIMELINE_HEIGHT: f32 = 64.0;

/// Where a clip is being looked at: the playhead, whether it plays on, and a cue being dragged.
/// A clip plays from the moment it is shown, as a body plays it, until the designer stops it.
pub struct Scrub {
    pub playhead: Millis,
    pub playing: bool,
    pub drag: Option<CueDrag>,
    /// Where the playhead went from and to this frame as it played or stepped on, counted on past
    /// the clip's end when it looped.
    passed: Option<(Millis, Millis)>,
}

impl Default for Scrub {
    fn default() -> Scrub {
        Scrub {
            playhead: Millis(0.0),
            playing: true,
            drag: None,
            passed: None,
        }
    }
}

impl Scrub {
    /// Back to the start of a clip, playing.
    pub fn restart(&mut self) {
        *self = Scrub::default();
    }

    /// The `cues` of a clip `length` long the playhead crossed this frame, as the game crosses a
    /// body's.
    pub fn crossed(&self, cues: &[Cue], length: Millis) -> Vec<Cue> {
        self.passed.map_or_else(Vec::new, |(from, to)| {
            crossed_cues(cues, length, false, Some(from), to)
                .into_iter()
                .cloned()
                .collect()
        })
    }
}

/// A cue being dragged along the timeline, and the cues as the drag has placed them.
pub struct CueDrag {
    pub cue: usize,
    pub cues: Vec<Cue>,
}

/// Keys every timeline answers, and what the status bar says of them.
pub const KEYS: &str = "Space pauses or plays · , . step 10 ms · A apex here · S step here · drag a cue · right-click removes it";

/// Moves the playhead on while the clip plays, and as the keys ask.
pub fn advance(ui: &mut Ui, scrub: &mut Scrub, length: Option<Millis>) {
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let typing = ui.ctx().text_edit_focused();
    let (play, back, on) = if typing {
        (false, false, false)
    } else {
        ui.input_mut(|i| {
            (
                EditorKey::PlayPause.consume(i),
                EditorKey::StepBack.consume(i),
                EditorKey::StepOn.consume(i),
            )
        })
    };
    scrub.passed = None;
    let Some(length) = length else {
        return;
    };
    if play {
        scrub.playing = !scrub.playing;
    }
    if back {
        scrub.playing = false;
        scrub.playhead = Millis((scrub.playhead.0 - STEP.0).max(0.0));
    }
    if on {
        scrub.playing = false;
        let from = scrub.playhead;
        scrub.playhead = Millis((from.0 + STEP.0).min(length.0));
        scrub.passed = Some((from, scrub.playhead));
    }
    if scrub.playing {
        let from = scrub.playhead;
        let to = Millis(from.0 + dt * 1000.0);
        scrub.passed = Some((from, to));
        scrub.playhead = Millis(to.0 % length.0.max(1.0));
        ui.ctx().request_repaint();
    }
    scrub.playhead = Millis(scrub.playhead.0.min(length.0));
}

/// The clip laid out large: cues to drag, the playhead to scrub, and the keys that mark moments.
/// Gives back the cues as an edit left them, in order of time, once it lands.
pub fn timeline(
    ui: &mut Ui,
    scrub: &mut Scrub,
    length: Millis,
    cues: &[Cue],
    takes_apex: bool,
) -> Option<Vec<Cue>> {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), TIMELINE_HEIGHT),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    let working: Vec<Cue> = scrub
        .drag
        .as_ref()
        .map_or_else(|| cues.to_vec(), |drag| drag.cues.clone());
    strip(&painter, rect, length, &working, Some(scrub.playhead), true);
    let pointer = response
        .hover_pos()
        .or_else(|| response.interact_pointer_pos());
    let at_pointer = pointer.map(|pointer| time_at(rect, length, pointer.x));
    let hot = pointer.and_then(|pointer| {
        working
            .iter()
            .position(|cue| (x_of(rect, length, cue.at) - pointer.x).abs() <= GRIP)
    });
    for (index, cue) in working.iter().enumerate() {
        let lit = hot == Some(index) || scrub.drag.as_ref().is_some_and(|drag| drag.cue == index);
        if lit {
            let x = x_of(rect, length, cue.at);
            painter.circle_stroke(
                Pos2::new(x, rect.center().y),
                GRIP,
                Stroke::new(1.0, theme::ACCENT),
            );
        }
    }
    let mut landed: Option<Vec<Cue>> = None;
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(index) = hot
    {
        scrub.drag = Some(CueDrag {
            cue: index,
            cues: working.clone(),
        });
    }
    if response.dragged_by(egui::PointerButton::Primary) {
        match (&mut scrub.drag, at_pointer) {
            (Some(drag), Some(at)) => {
                drag.cues[drag.cue].at = at;
            }
            (None, Some(at)) => {
                scrub.playing = false;
                scrub.playhead = at;
            }
            _ => {}
        }
        ui.ctx().request_repaint();
    }
    if response.clicked()
        && let Some(at) = at_pointer
    {
        scrub.playing = false;
        scrub.playhead = at;
    }
    if response.drag_stopped()
        && let Some(drag) = scrub.drag.take()
    {
        landed = Some(drag.cues);
    }
    if response.secondary_clicked()
        && let Some(index) = hot
    {
        let mut cues = working.clone();
        cues.remove(index);
        landed = Some(cues);
    }
    let typing = ui.ctx().text_edit_focused();
    let (apex, step) = if typing {
        (false, false)
    } else {
        ui.input_mut(|i| (EditorKey::ApexCue.consume(i), EditorKey::StepCue.consume(i)))
    };
    let playhead = scrub.playhead;
    if apex && takes_apex {
        let mut cues = working.clone();
        match cues.iter_mut().find(|cue| cue.apex) {
            Some(cue) => cue.at = playhead,
            None => cues.push(Cue {
                at: playhead,
                step: false,
                apex: true,
                sound: None,
            }),
        }
        landed = Some(cues);
    }
    if step {
        let mut cues = working.clone();
        match cues
            .iter()
            .position(|cue| cue.step && (cue.at.0 - playhead.0).abs() <= STEP.0 * 1.5)
        {
            Some(index) => {
                let cue = &mut cues[index];
                cue.step = false;
                if !cue.apex && cue.sound.is_none() {
                    cues.remove(index);
                }
            }
            None => cues.push(Cue {
                at: playhead,
                step: true,
                apex: false,
                sound: None,
            }),
        }
        landed = Some(cues);
    }
    ui.add_space(4.0);
    form::hint(
        ui,
        &format!("{:.0} ms · {} cues", playhead.0, working.len()),
    );
    landed.map(|mut cues| {
        cues.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
        cues
    })
}

/// A clip's length as a band, with its cues marked: an apex as a triangle, a step as a dot, a
/// sound as a note.
pub fn strip(
    painter: &egui::Painter,
    rect: Rect,
    length: Millis,
    cues: &[Cue],
    playhead: Option<Millis>,
    selected: bool,
) {
    painter.rect_filled(rect, 3.0, theme::INPUT);
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(
            1.0,
            if selected {
                theme::ACCENT
            } else {
                theme::BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    let ticks = (length.0 / 100.0).floor() as usize;
    for tick in 1..=ticks {
        let x = x_of(rect, length, Millis(tick as f32 * 100.0));
        let tall = tick % 10 == 0;
        painter.line_segment(
            [
                Pos2::new(x, rect.bottom() - if tall { 8.0 } else { 4.0 }),
                Pos2::new(x, rect.bottom()),
            ],
            Stroke::new(1.0, theme::BORDER),
        );
    }
    for cue in cues {
        let x = x_of(rect, length, cue.at);
        let middle = rect.center().y;
        painter.line_segment(
            [
                Pos2::new(x, rect.top() + 2.0),
                Pos2::new(x, rect.bottom() - 2.0),
            ],
            Stroke::new(1.0, ui::ink::wash(cue_color(cue), 0.6)),
        );
        if cue.apex {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(x - 5.0, middle + 5.0),
                    Pos2::new(x + 5.0, middle + 5.0),
                    Pos2::new(x, middle - 5.0),
                ],
                theme::DANGER,
                Stroke::NONE,
            ));
        }
        if cue.step {
            painter.circle_filled(Pos2::new(x, middle), 3.5, theme::TEXT);
        }
        if cue.sound.is_some() {
            Mark::Music.paint(
                painter,
                Rect::from_center_size(Pos2::new(x, rect.top() + 7.0), Vec2::splat(10.0)),
                theme::OK,
            );
        }
    }
    if let Some(playhead) = playhead {
        let x = x_of(rect, length, playhead);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(2.0, theme::ACCENT),
        );
    }
}

/// The cues of `cues` that still lie within a clip `length` long.
pub fn fitting(cues: &[Cue], length: Millis) -> Vec<Cue> {
    cues.iter()
        .filter(|cue| cue.at <= length)
        .cloned()
        .collect()
}

fn cue_color(cue: &Cue) -> Color32 {
    if cue.apex {
        theme::DANGER
    } else if cue.sound.is_some() {
        theme::OK
    } else {
        theme::TEXT
    }
}

fn x_of(rect: Rect, length: Millis, at: Millis) -> f32 {
    rect.left() + 6.0 + (rect.width() - 12.0) * (at.0 / length.0.max(1.0)).clamp(0.0, 1.0)
}

fn time_at(rect: Rect, length: Millis, x: f32) -> Millis {
    let ratio = ((x - rect.left() - 6.0) / (rect.width() - 12.0).max(1.0)).clamp(0.0, 1.0);
    Millis((ratio * length.0 / STEP.0).round() * STEP.0)
}
