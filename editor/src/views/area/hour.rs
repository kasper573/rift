//! The hour the editor shows its areas at: none, as they are authored, or an hour of the game's
//! day, held or played through, under which everything that follows the sky is shown as a player
//! finds it then.

use std::sync::Arc;

use engine::core::content::{Content, FolderReader};
use engine::core::units::{HourOfDay, Seconds};
use engine::systems::GameSettings;
use engine::systems::area::prop::PropId;
use engine::systems::area::{AreaModule, DayCycle, TimeOfDay, Weather};
use engine::systems::npc::{AbsenceSpanAt, NpcDef, NpcId, absence};
use engine::systems::script::{
    Condition, Sky, SkyCondition, SkyNow, condition as script_condition, sky,
};
use engine::systems::spawner::{SpawnShare, Spawns};
use strum::VariantArray;
use ui::egui::{self, Align, Layout, Rect as ERect, Ui, Vec2};
use ui::glyph::{self, Mark};
use ui::input::NumberField;
use ui::{chip, controls, theme};

use crate::app::Editor;
use crate::keymap::EditorKey;

/// The hour every area view is shown at, and whether the day plays on from it.
#[derive(Default)]
pub struct DayPreview {
    hour: Option<HourOfDay>,
    playing: bool,
    pace: DayPace,
    pointed: bool,
    answered: Option<(u64, Sky, Arc<SkyNow>)>,
}

/// How fast a played day runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, VariantArray)]
enum DayPace {
    #[default]
    Game,
    Minute,
    Glance,
}

impl DayPace {
    fn name(self) -> &'static str {
        match self {
            DayPace::Game => "at the game's pace",
            DayPace::Minute => "a day a minute",
            DayPace::Glance => "a day in ten seconds",
        }
    }

    fn day_takes(self, cycle: &DayCycle) -> Seconds {
        match self {
            DayPace::Game => cycle.day_length,
            DayPace::Minute => Seconds(60.0),
            DayPace::Glance => Seconds(10.0),
        }
    }
}

impl DayPreview {
    /// The hour of the game's day the areas are shown at; none in their authored light.
    pub fn hour(&self) -> Option<HourOfDay> {
        self.hour
    }

    /// Whether the pointer is over the hour's own controls, which then take the keys.
    pub fn pointed(&self) -> bool {
        self.pointed
    }

    /// The sky an area under `weather` is shown under, answered for a game of `content` read
    /// as the project's `generation`.
    pub fn shown(&mut self, content: &Content, generation: u64, weather: Weather) -> ShownSky {
        let sky = Sky {
            hour: self.hour,
            weather,
            rain: 0.0,
        };
        let now = match &self.answered {
            Some((answered, under, now)) if (*answered, *under) == (generation, sky) => now.clone(),
            _ => {
                let now = Arc::new(SkyNow::under(content, sky));
                self.answered = Some((generation, sky, now.clone()));
                now
            }
        };
        ShownSky {
            content: content.clone(),
            sky,
            now,
        }
    }

    /// Shows the areas at `hour` o'clock, held there.
    pub fn show_at(&mut self, hour: HourOfDay) {
        self.hour = Some(hour);
        self.playing = false;
    }

    /// Moves the hour shown by `hours`, from noon when the areas stood as authored.
    pub fn step(&mut self, hours: f32) {
        let from = self.hour.unwrap_or(HourOfDay::NOON);
        self.hour = Some(HourOfDay::wrapped(from.0.round() + hours));
    }

    /// Moves the hour shown to where the next time of day begins, or the one before.
    pub fn turn(&mut self, content: &Content, later: bool) {
        let Some(cycle) = cycle(content) else {
            return;
        };
        let from = self.hour.unwrap_or(HourOfDay::NOON).0;
        let apart = |begins: f32| {
            if later {
                (begins - from).rem_euclid(24.0)
            } else {
                (from - begins).rem_euclid(24.0)
            }
        };
        let next = cycle
            .times
            .iter()
            .map(|time| time.from.0)
            .filter(|begins| apart(*begins) > f32::EPSILON)
            .min_by(|a, b| apart(*a).total_cmp(&apart(*b)));
        if let Some(begins) = next {
            self.hour = Some(HourOfDay(begins));
        }
    }

    pub fn as_authored(&mut self) {
        self.hour = None;
        self.playing = false;
    }

    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        if self.playing && self.hour.is_none() {
            self.hour = Some(HourOfDay::NOON);
        }
    }

    /// Runs a played day on by `seconds`; whether it plays.
    pub fn advance(&mut self, content: &Content, seconds: f32) -> bool {
        let (true, Some(hour), Some(cycle)) = (self.playing, self.hour, cycle(content)) else {
            return false;
        };
        let day = self.pace.day_takes(cycle).0.max(1.0);
        self.hour = Some(HourOfDay::wrapped(hour.0 + seconds / day * 24.0));
        true
    }
}

/// The sky an area is shown under, for what follows it to ask what holds.
pub struct ShownSky {
    content: Content,
    sky: Sky,
    now: Arc<SkyNow>,
}

/// What the sky shown makes of a module that follows it.
pub struct SkyFollowing {
    /// Whether the module is not there under the sky shown.
    pub away: bool,
    /// What it follows, in words.
    pub words: String,
    /// The sign that stands for an npc away now.
    pub sign: Option<PropId>,
}

impl ShownSky {
    pub fn sky(&self) -> Sky {
        self.sky
    }

    /// How much of what follows `when` is there under the sky shown.
    pub fn share(&self, when: Option<&SkyCondition>) -> f32 {
        self.now.share(when)
    }

    /// The time of day that holds, in a game with any.
    pub fn time(&self) -> Option<&TimeOfDay> {
        cycle(&self.content)?.time_at(self.sky.hour.unwrap_or(HourOfDay::NOON))
    }

    /// Whether a spawner draws `share`, its ids among `reader`'s, under the sky shown: not an
    /// npc that is away, nor a share none of which is there.
    pub fn spawns(&self, reader: &FolderReader, share: &SpawnShare) -> bool {
        let away = match share.what {
            Spawns::Npc { npc } => self.absent(reader, npc).is_some(),
            Spawns::Resource { .. } => false,
        };
        !away && self.share(share.when.as_ref()) > 0.0
    }

    /// The npc `npc`, an id among `reader`'s, and the span of its absence it is away for under
    /// the sky shown, if it is away.
    fn absent(&self, reader: &FolderReader, npc: NpcId) -> Option<(NpcId, AbsenceSpanAt)> {
        let npc = self.content.by_path::<NpcDef>(reader.path_of(npc)?)?;
        let span = npc
            .get(&self.content)
            .absence()?
            .span_during(&self.time()?.name)?;
        Some((npc, span))
    }

    /// What the sky makes of `module`, its ids among `reader`'s; none when it follows no sky.
    pub fn following(&self, reader: &FolderReader, module: &AreaModule) -> Option<SkyFollowing> {
        match module {
            AreaModule::Resident(resident) => {
                let npc = reader
                    .path_of(resident.npc)
                    .and_then(|npc| self.content.by_path::<NpcDef>(npc));
                let absence = npc
                    .and_then(|npc| npc.get(&self.content).absence())
                    .filter(|absence| !absence.spans.is_empty());
                let span = absence
                    .zip(self.time())
                    .and_then(|(absence, time)| absence.span_during(&time.name));
                let shown = resident
                    .shown
                    .as_ref()
                    .and_then(|shown| Some((shown, self.holds(shown)?)));
                if absence.is_none() && shown.is_none() {
                    return None;
                }
                let away_during = absence.map(|absence| {
                    let times: Vec<&str> = absence
                        .spans
                        .iter()
                        .map(|span| span.time.0.as_str())
                        .collect();
                    format!("away during {}", times.join(", "))
                });
                let shown_when = shown.map(|(shown, _)| self.shown_when(shown));
                Some(SkyFollowing {
                    away: span.is_some() || shown.is_some_and(|(_, holds)| !holds),
                    words: away_during
                        .into_iter()
                        .chain(shown_when)
                        .collect::<Vec<_>>()
                        .join("; "),
                    sign: npc
                        .zip(span)
                        .and_then(|(npc, span)| absence::sign_of(&self.content, npc, span)),
                })
            }
            AreaModule::Object(object) => {
                let shown = object.shown.as_ref()?;
                let holds = self.holds(shown)?;
                Some(SkyFollowing {
                    away: !holds,
                    words: self.shown_when(shown),
                    sign: None,
                })
            }
            module => {
                let whens = module.whens();
                if whens.is_empty() {
                    return None;
                }
                let dark = matches!(module, AreaModule::Light(_) | AreaModule::Fog(_))
                    && whens.iter().all(|(_, when)| self.share(Some(when)) <= 0.0);
                let mut words: Vec<String> = whens
                    .iter()
                    .map(|(_, when)| {
                        script_condition::describe(&self.content, when.condition()).to_string()
                    })
                    .collect();
                words.dedup();
                Some(SkyFollowing {
                    away: dark,
                    words: format!("when {}", words.join("; when ")),
                    sign: None,
                })
            }
        }
    }

    /// Whether `shown`, a condition of who sees something, holds under the sky shown; none when
    /// it reads who asks, so no hour answers it.
    fn holds(&self, shown: &Condition) -> Option<bool> {
        if sky::unshared_read(shown).is_some() {
            return None;
        }
        let shares = SkyCondition::from(shown.clone())
            .through(&self.content, [self.sky])
            .ok()?;
        Some(shares.first().is_some_and(|share| *share > 0.0))
    }

    fn shown_when(&self, shown: &Condition) -> String {
        format!(
            "shown when {}",
            script_condition::describe(&self.content, shown)
        )
    }
}

/// The hour the areas are shown at, in the corner of an area view's map `canvas`: dragged or
/// typed, played, and put back to the light the areas are authored in.
pub(super) fn bar(editor: &mut Editor, ui: &mut Ui, canvas: ERect) {
    let Some(content) = editor
        .project
        .as_ref()
        .and_then(|project| project.content.clone())
    else {
        return;
    };
    let Some(cycle) = cycle(&content) else {
        return;
    };
    let seconds = ui.input(|input| input.stable_dt).min(0.1);
    let day = &mut editor.views.area.day;
    if day.advance(&content, seconds) {
        ui.ctx().request_repaint();
    }
    let corner = ERect::from_min_max(
        canvas.min + Vec2::new(8.0, 8.0),
        canvas.max - Vec2::new(8.0, 8.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(corner)
            .layout(Layout::right_to_left(Align::Min)),
    );
    let shown = ui::slip::show(&mut child, |ui| {
        ui.horizontal(|ui| {
            let (mark, does) = if day.playing {
                (Mark::Pause, "Pause the day")
            } else {
                (Mark::Play, "Play the day")
            };
            if glyph::button(ui, mark, 13.0, theme::TEXT, does)
                .on_hover_text(does)
                .clicked()
            {
                day.toggle_play();
            }
            match day.hour {
                Some(hour) => {
                    let mut typed = hour.0;
                    if controls::slider(ui, &mut typed, 0.0..=24.0).changed()
                        | NumberField::new(&mut typed)
                            .range(0.0..=24.0)
                            .unit("h")
                            .width(HOUR_WIDTH)
                            .show(ui)
                    {
                        day.hour = Some(HourOfDay::wrapped(typed));
                    }
                    if let Some(time) = cycle.time_at(HourOfDay::wrapped(typed)) {
                        chip::swatch(ui, time.name.0.as_str(), time.color.into());
                    }
                    if day.playing {
                        controls::combo("day-pace")
                            .selected_text(day.pace.name())
                            .show_ui(ui, |ui| {
                                for &pace in DayPace::VARIANTS {
                                    ui.selectable_value(&mut day.pace, pace, pace.name());
                                }
                            });
                    }
                    if controls::button(ui, "As authored")
                        .on_hover_text(format!(
                            "The light the area is authored in ({})",
                            EditorKey::AsAuthored.name()
                        ))
                        .clicked()
                    {
                        day.as_authored();
                    }
                }
                None => {
                    if controls::button(ui, "As authored")
                        .on_hover_text(format!(
                            "Shown in its authored light. Pick an hour of the game's day to see \
                             what follows it ({} and {})",
                            EditorKey::HourEarlier.name(),
                            EditorKey::HourLater.name()
                        ))
                        .clicked()
                    {
                        day.step(0.0);
                    }
                }
            }
        });
    });
    day.pointed = shown.response.contains_pointer();
    if day.pointed && ui.input_mut(|input| EditorKey::PlayDay.consume(input)) {
        day.toggle_play();
    }
}

/// The keys that move the hour the areas are shown at.
pub(super) fn keys(editor: &mut Editor, input: &egui::InputState) {
    let Some(content) = editor
        .project
        .as_ref()
        .and_then(|project| project.content.clone())
    else {
        return;
    };
    let day = &mut editor.views.area.day;
    if EditorKey::TurnEarlier.pressed(input) {
        day.turn(&content, false);
    } else if EditorKey::HourEarlier.pressed(input) {
        day.step(-1.0);
    }
    if EditorKey::TurnLater.pressed(input) {
        day.turn(&content, true);
    } else if EditorKey::HourLater.pressed(input) {
        day.step(1.0);
    }
    if EditorKey::AsAuthored.pressed(input) {
        day.as_authored();
    }
}

fn cycle(content: &Content) -> Option<&DayCycle> {
    content.try_built::<GameSettings>()?.daylight.as_ref()
}

const HOUR_WIDTH: f32 = 64.0;
