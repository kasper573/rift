use std::path::Path;
use std::time::SystemTime;

use driver::{Footage, FootageOverlay, InsetCut, reading, seconds};
use engine::core::time::Seconds;
use engine::systems::probe::{ProbeSnapshot, ProbeVoice};

// A chapter's cut of the screen it films: a title card, captions that pace the chapter, and
// panels for what the picture can't show. The chapter plays as fast as the game lets it; only a
// caption holds it, until a viewer could have read it.
pub struct Film {
    title: String,
    summary: String,
    rolled: SystemTime,
    card_until: SystemTime,
    read_until: SystemTime,
    captions: Vec<Caption>,
    panels: Vec<Panel>,
    mixing: bool,
    insets: Vec<Inset>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Bottom,
    // Clear of whatever the game draws along the bottom edge.
    Top,
}

impl Film {
    pub fn roll(title: &str, summary: &str) -> Film {
        let rolled = SystemTime::now();
        let card_until = rolled + reading(&format!("{title} {summary}"));
        Film {
            title: title.to_owned(),
            summary: summary.to_owned(),
            rolled,
            card_until,
            read_until: card_until,
            captions: Vec::new(),
            panels: Vec::new(),
            mixing: false,
            insets: Vec::new(),
        }
    }

    // Until then the caption that's up is still being read.
    pub fn read_until(&self) -> SystemTime {
        self.read_until
    }

    pub fn caption(&mut self, text: &str, edge: Edge) {
        let now = SystemTime::now();
        self.captions.push(Caption {
            text: text.to_owned(),
            edge,
            from: now,
        });
        self.read_until = now + reading(text);
    }

    pub fn show_mixer(&mut self) {
        self.mixing = true;
    }

    pub fn hide_mixer(&mut self) {
        self.mixing = false;
        self.close_panel(SystemTime::now());
    }

    // Redraws the mixer panel whenever what it shows changes.
    pub fn watch(&mut self, snapshot: &ProbeSnapshot) {
        if !self.mixing {
            return;
        }
        let shown = mixer_panel(&snapshot.soundscape.voices);
        if self
            .panels
            .last()
            .is_some_and(|panel| panel.until.is_none() && panel.text == shown)
        {
            return;
        }
        let now = SystemTime::now();
        self.close_panel(now);
        self.panels.push(Panel {
            text: shown,
            from: now,
            until: None,
        });
    }

    pub fn inset(&mut self, footage: Footage, label: &str, from: SystemTime) {
        self.insets.push(Inset {
            footage,
            label: label.to_owned(),
            from,
            until: SystemTime::now(),
        });
    }

    pub fn render(&mut self, footage: &Footage, scratch: &Path, out: &Path) {
        let subtitles = scratch.join("film.ass");
        std::fs::write(&subtitles, self.subtitles()).expect("a subtitle script");
        let overlay = FootageOverlay {
            origin: self.rolled,
            subtitles,
            blur_until: seconds(self.card_until, self.rolled),
            insets: std::mem::take(&mut self.insets)
                .into_iter()
                .map(|inset| InsetCut {
                    from: seconds(inset.from, self.rolled),
                    until: seconds(inset.until, self.rolled),
                    at: INSET_AT,
                    size: INSET_SIZE,
                    footage: inset.footage,
                })
                .collect(),
        };
        footage.render(out, Some(&overlay));
        for inset in &overlay.insets {
            inset.footage.discard();
        }
        println!("recorded {}", out.display());
    }

    fn close_panel(&mut self, at: SystemTime) {
        if let Some(open) = self.panels.last_mut().filter(|panel| panel.until.is_none()) {
            open.until = Some(at);
        }
    }

    fn subtitles(&self) -> String {
        let at = |time: SystemTime| ass_time(seconds(time, self.rolled));
        let card_until = at(self.card_until);
        let mut script = String::from(SCRIPT_HEAD);
        let mut event = |layer: u32, from: &str, until: &str, style: &str, text: &str| {
            script += &format!("Dialogue: {layer},{from},{until},{style},,0,0,0,,{text}\n");
        };
        let start = ass_time(Seconds(0.0));
        event(0, &start, &card_until, "Veil", VEIL);
        let title = format!("{{\\pos(640,330)}}{}", escape(&self.title));
        let summary = format!("{{\\pos(640,400)}}{}", escape(&self.summary));
        event(1, &start, &card_until, "Title", &title);
        event(1, &start, &card_until, "Summary", &summary);
        for (index, caption) in self.captions.iter().enumerate() {
            let until = self
                .captions
                .get(index + 1)
                .map_or(END.to_owned(), |next| at(next.from));
            let style = match caption.edge {
                Edge::Bottom => "Caption",
                Edge::Top => "CaptionTop",
            };
            event(0, &at(caption.from), &until, style, &escape(&caption.text));
        }
        for panel in &self.panels {
            let until = panel.until.map_or(END.to_owned(), at);
            event(0, &at(panel.from), &until, "Panel", &panel.text);
        }
        for inset in &self.insets {
            let label = format!(
                "{{\\pos({},{})}}{}",
                INSET_AT.0 + 3,
                INSET_AT.1 + 3,
                escape(&inset.label)
            );
            event(2, &at(inset.from), &at(inset.until), "Label", &label);
        }
        script
    }
}

struct Caption {
    text: String,
    edge: Edge,
    from: SystemTime,
}

struct Panel {
    text: String,
    from: SystemTime,
    until: Option<SystemTime>,
}

struct Inset {
    footage: Footage,
    label: String,
    from: SystemTime,
    until: SystemTime,
}

// The other screen's footage sits top left, under its label.
const INSET_AT: (u32, u32) = (16, 168);
const INSET_SIZE: (u32, u32) = (416, 234);
const END: &str = "9:59:59.00";
const METER_CELLS: usize = 12;
const METER_RANGE_DB: f32 = 48.0;
const VEIL: &str = "{\\p1}m 0 0 l 1280 0 1280 720 0 720{\\p0}";

// ASS colors are &HAABBGGRR, with 00 opaque.
const SCRIPT_HEAD: &str = "[Script Info]
ScriptType: v4.00+
PlayResX: 1280
PlayResY: 720
WrapStyle: 0
ScaledBorderAndShadow: yes

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColor, SecondaryColor, OutlineColor, BackColor, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Caption,DejaVu Sans,24,&H00FFFFFF,&H00FFFFFF,&H48000000,&H48000000,0,0,0,0,100,100,0,0,3,10,0,2,190,190,48,1
Style: CaptionTop,DejaVu Sans,24,&H00FFFFFF,&H00FFFFFF,&H48000000,&H48000000,0,0,0,0,100,100,0,0,3,10,0,8,190,190,48,1
Style: Veil,DejaVu Sans,20,&H66100A08,&H66100A08,&H66100A08,&H66100A08,0,0,0,0,100,100,0,0,1,0,0,7,0,0,0,1
Style: Title,DejaVu Sans,64,&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,0,0,5,0,0,0,1
Style: Summary,DejaVu Sans,26,&H00E1D5CB,&H00E1D5CB,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,5,0,0,0,1
Style: Panel,DejaVu Sans Mono,15,&H00FFFFFF,&H00FFFFFF,&H48000000,&H48000000,0,0,0,0,100,100,0,0,3,10,0,9,0,72,40,1
Style: Label,DejaVu Sans,16,&H00FFFFFF,&H00FFFFFF,&H48000000,&H48000000,-1,0,0,0,100,100,0,0,3,5,0,7,0,0,0,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
";

fn mixer_panel(voices: &[ProbeVoice]) -> String {
    let mut voices: Vec<&ProbeVoice> = voices.iter().collect();
    voices.sort_by(|a, b| a.channel.cmp(&b.channel).then_with(|| a.src.cmp(&b.src)));
    let rows: Vec<String> = voices
        .iter()
        .map(|voice| {
            let name = Path::new(&voice.src)
                .file_stem()
                .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
            let category = format!("{:?}", voice.category).to_lowercase();
            format!(
                "{:>2}  {name:<14.14} {category:<9} {}",
                voice.channel,
                meter(voice.heard)
            )
        })
        .collect();
    let body = if rows.is_empty() {
        "silence".to_owned()
    } else {
        rows.join("\\N")
    };
    format!("{{\\b1}}Soundscape channels{{\\b0}}\\N{body}")
}

// Loudness is heard in decibels, so the bar spans METER_RANGE_DB of them down from full scale.
fn meter(amplitude: f32) -> String {
    let decibels = 20.0 * amplitude.max(1e-6).log10();
    let level = (1.0 + decibels / METER_RANGE_DB).clamp(0.0, 1.0);
    let lit = (level * METER_CELLS as f32).round() as usize;
    "█".repeat(lit) + &"░".repeat(METER_CELLS - lit)
}

fn ass_time(Seconds(at): Seconds) -> String {
    let centis = (at.max(0.0) * 100.0).round() as u64;
    format!(
        "{}:{:02}:{:02}.{:02}",
        centis / 360_000,
        centis / 6_000 % 60,
        centis / 100 % 60,
        centis % 100
    )
}

// Braces open override tags in a subtitle script.
fn escape(text: &str) -> String {
    text.replace('{', "(")
        .replace('}', ")")
        .replace('\n', "\\N")
}
