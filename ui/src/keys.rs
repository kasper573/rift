use egui::{FontFamily, InputState, Key, KeyboardShortcut, Modifiers};

use crate::face::Face;
use crate::glyph::{Icon, phosphor};

/// Takes what's typed or picked.
pub const CONFIRM: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::Enter);
/// Leaves without taking it.
pub const CANCEL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::Escape);

pub fn pressed(input: &InputState, key: KeyboardShortcut) -> bool {
    input.key_pressed(key.logical_key) && input.modifiers.matches_exact(key.modifiers)
}

/// The text pasted this frame. A host may hand a paste over as `Paste`, or, as bevy_egui does on
/// desktops, as the clipboard's text typed in the frame the paste shortcut is pressed.
pub fn pasted(input: &InputState) -> Option<String> {
    let shortcut = input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.command
        )
    });
    input.events.iter().find_map(|event| match event {
        egui::Event::Paste(text) => Some(text.clone()),
        egui::Event::Text(text) if shortcut => Some(text.clone()),
        _ => None,
    })
}

/// A binding as a cap draws it: the modifiers held, then the button or key.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyCap {
    pub held: Vec<String>,
    pub face: CapFace,
    /// What the cap says in words: its tooltip, and what a screen reader reads.
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CapFace {
    Text(String),
    Mouse {
        glyph: MouseGlyph,
        presses: u8,
        drag: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseGlyph {
    Left,
    Right,
    Middle,
    WheelUp,
    WheelDown,
    Back,
    Forward,
}

/// A piece of a cap's face set in one font: words in the cap's own, glyphs in the icon font.
#[derive(Clone, Debug, PartialEq)]
pub struct CapRun {
    pub text: String,
    pub family: FontFamily,
    /// The run's size as a share of the cap's letters.
    pub scale: f32,
}

impl KeyCap {
    /// A cap of words alone, named by them.
    pub fn text(words: impl Into<String>) -> KeyCap {
        let words = words.into();
        KeyCap {
            held: Vec::new(),
            name: words.clone(),
            face: CapFace::Text(words),
        }
    }

    /// Each cap it is drawn as, left to right: one per modifier held, then its face.
    pub fn faces(&self) -> impl Iterator<Item = CapFace> + '_ {
        self.held
            .iter()
            .map(|held| CapFace::Text(held.clone()))
            .chain(std::iter::once(self.face.clone()))
    }
}

impl CapFace {
    pub fn runs(&self) -> Vec<CapRun> {
        let words = |text: String| CapRun {
            text,
            family: Face::Mono.family(),
            scale: 1.0,
        };
        let glyph = |icon: Icon| CapRun {
            text: icon.glyph.to_owned(),
            family: icon.weight.family(),
            scale: GLYPH_SCALE,
        };
        match self {
            CapFace::Text(text) => vec![words(text.clone())],
            CapFace::Mouse {
                glyph: mouse,
                presses,
                drag,
            } => {
                let mut runs: Vec<CapRun> = mouse.icons().into_iter().map(glyph).collect();
                if *drag {
                    runs.push(glyph(Icon::regular(phosphor::ARROWS_LEFT_RIGHT)));
                }
                if *presses > 1 {
                    runs.push(words(format!("×{presses}")));
                }
                runs
            }
        }
    }
}

impl MouseGlyph {
    fn icons(self) -> Vec<Icon> {
        let mouse = Icon::regular;
        let caret = Icon::fill;
        match self {
            MouseGlyph::Left => vec![mouse(phosphor::MOUSE_LEFT_CLICK)],
            MouseGlyph::Right => vec![mouse(phosphor::MOUSE_RIGHT_CLICK)],
            MouseGlyph::Middle => vec![mouse(phosphor::MOUSE_MIDDLE_CLICK)],
            MouseGlyph::WheelUp => vec![mouse(phosphor::MOUSE_SCROLL), caret(phosphor::CARET_UP)],
            MouseGlyph::WheelDown => {
                vec![mouse(phosphor::MOUSE_SCROLL), caret(phosphor::CARET_DOWN)]
            }
            MouseGlyph::Back => vec![caret(phosphor::CARET_LEFT), mouse(phosphor::MOUSE_SIMPLE)],
            MouseGlyph::Forward => {
                vec![mouse(phosphor::MOUSE_SIMPLE), caret(phosphor::CARET_RIGHT)]
            }
        }
    }
}

const GLYPH_SCALE: f32 = 1.25;
