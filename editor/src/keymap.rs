use strum::VariantArray;
use ui::egui::os::OperatingSystem;
use ui::egui::{InputState, Key, KeyboardShortcut, ModifierNames, Modifiers};

/// Every input the editor answers to. Handlers ask a key whether it was pressed or is held, and
/// every hint, menu and the key sheet names it, so a key is bound in one place only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, VariantArray)]
pub enum EditorKey {
    Search,
    Save,
    Undo,
    Redo,
    CloseTab,
    Quit,
    KeySheet,
    FrameRate,
    PlayFromStart,
    PlayHere,
    StopPlay,
    PlayedCharacter,
    Rename,
    NewAgain,
    CopyPath,
    Usages,
    Duplicate,
    Delete,
    Confirm,
    NewLine,
    OpenInNewTab,
    Cancel,
    Move,
    Pick,
    Copy,
    Paste,
    SelectAll,
    ToggleSelection,
    ExtendSelection,
    FineStep,
    Unsnapped,
    Pan,
    Zoom,
    ContextMenu,
    FirstLine,
    LastLine,
    SelectTool,
    SculptTool,
    PaintTool,
    PlaceTool,
    ScatterTool,
    ModuleTool,
    TurnClockwise,
    TurnCounter,
    NavmeshOverlay,
    ContoursOverlay,
    FootprintsOverlay,
    BrushSmaller,
    BrushLarger,
    FrameSelection,
    TopDown,
    SelectAtCursor,
    PasteInPlace,
    MakeBlueprint,
    Listen,
    FilterScene,
    OpenBlueprint,
    Group,
    Ungroup,
    NewFolder,
    Reorder,
    Cut,
    Hide,
    Lock,
    ShowOnly,
    ShowEverything,
    UnlockEverything,
    Shrink,
    Grow,
    Pin,
    Route,
    PlayPause,
    PreviousFrame,
    NextFrame,
    Mute,
    ApexCue,
    SoundCue,
    StepCue,
    InsertFrame,
    FrameShorter,
    FrameLonger,
    FrameEarlier,
    FrameLater,
    RunScript,
    Palette,
    Outline,
    AddChoice,
    Link,
    NextCell,
    FillDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, VariantArray)]
pub enum KeyScope {
    Everywhere,
    Area,
    SceneGraph,
    WorldMap,
    Model,
    Script,
    Spreadsheet,
}

impl KeyScope {
    pub fn title(self) -> &'static str {
        match self {
            KeyScope::Everywhere => "Everywhere",
            KeyScope::Area => "Area view",
            KeyScope::SceneGraph => "Scene graph",
            KeyScope::WorldMap => "World map",
            KeyScope::Model => "Model",
            KeyScope::Script => "Script",
            KeyScope::Spreadsheet => "Spreadsheet",
        }
    }

    pub fn keys(self) -> impl Iterator<Item = EditorKey> {
        EditorKey::VARIANTS
            .iter()
            .copied()
            .filter(move |key| key.scope() == self)
    }
}

impl EditorKey {
    /// Whether the key went down this frame, unless a chord on the same key with more modifiers
    /// held claims it, so Ctrl Shift Z never also counts as Ctrl Z.
    pub fn pressed(self, input: &InputState) -> bool {
        match self.binding() {
            Binding::Press(chords) => chords.iter().any(|chord| claims(chord, input)),
            Binding::Arrows(modifiers) => arrow(input, modifiers).is_some(),
            Binding::Digits => digit(input).is_some(),
            Binding::Hold(_) | Binding::HoldKey(_) => false,
        }
    }

    /// Like `pressed`, and takes the key from the frame's input so nothing else answers it.
    pub fn consume(self, input: &mut InputState) -> bool {
        let Binding::Press(chords) = self.binding() else {
            return self.pressed(input);
        };
        let pressed = chords.iter().any(|chord| claims(chord, input));
        if pressed {
            for chord in &chords {
                input.consume_shortcut(chord);
            }
        }
        pressed
    }

    pub fn held(self, input: &InputState) -> bool {
        match self.binding() {
            Binding::HoldKey(key) => input.key_down(key),
            _ => self.held_among(input.modifiers),
        }
    }

    /// Whether a modifier key is among `modifiers`, as recorded with a click.
    pub fn held_among(self, modifiers: Modifiers) -> bool {
        match self.binding() {
            Binding::Hold(held) => modifiers.contains(held),
            _ => false,
        }
    }

    /// The step an arrow key pressed this frame takes, one tile or cell along its axis.
    pub fn arrow(self, input: &InputState) -> Option<[i32; 2]> {
        match self.binding() {
            Binding::Arrows(modifiers) => arrow(input, modifiers),
            _ => None,
        }
    }

    /// Like `arrow`, and takes the arrow key from the frame's input so nothing else answers it.
    pub fn take_arrow(self, input: &mut InputState) -> Option<[i32; 2]> {
        let Binding::Arrows(modifiers) = self.binding() else {
            return None;
        };
        let (key, step) = ARROWS
            .iter()
            .find(|(key, _)| claims(&KeyboardShortcut::new(modifiers, *key), input))?;
        input.consume_shortcut(&KeyboardShortcut::new(modifiers, *key));
        Some(*step)
    }

    /// Which numbered entry, counted from 0, a digit key pressed this frame picks.
    pub fn digit(self, input: &InputState) -> Option<usize> {
        match self.binding() {
            Binding::Digits => digit(input),
            _ => None,
        }
    }

    pub fn name(self) -> String {
        let names = ModifierNames {
            concat: " ",
            ..ModifierNames::NAMES
        };
        let mac = OperatingSystem::from_target_os() == OperatingSystem::Mac;
        let held = |modifiers: Modifiers| names.format(&modifiers, mac);
        let chord = |modifiers: Modifiers, key: &str| {
            let held = held(modifiers);
            if held.is_empty() {
                key.to_owned()
            } else {
                format!("{held} {key}")
            }
        };
        match self.binding() {
            Binding::Press(chords) => chords.first().map_or_else(String::new, |first| {
                chord(first.modifiers, key_name(first.logical_key))
            }),
            Binding::Arrows(modifiers) => chord(modifiers, "Arrows"),
            Binding::Digits => format!("{} – {}", key_name(DIGITS[0]), key_name(DIGITS[8])),
            Binding::Hold(modifiers) => held(modifiers),
            Binding::HoldKey(key) => key_name(key).to_owned(),
        }
    }

    /// The name of the digit key that picks the numbered entry `index`, counted from 0.
    pub fn digit_name(index: usize) -> Option<&'static str> {
        DIGITS.get(index).map(|key| key_name(*key))
    }

    pub fn scope(self) -> KeyScope {
        use EditorKey::*;
        match self {
            Search | Save | Undo | Redo | CloseTab | Quit | KeySheet | FrameRate
            | PlayFromStart | PlayHere | StopPlay | PlayedCharacter | Rename | NewAgain
            | CopyPath | Usages | Duplicate | Delete | Confirm | NewLine | OpenInNewTab
            | Cancel | Move | Pick | Copy | Paste | SelectAll | ToggleSelection
            | ExtendSelection | FineStep | Unsnapped | Pan | Zoom | ContextMenu | FirstLine
            | LastLine => KeyScope::Everywhere,
            SelectTool | SculptTool | PaintTool | PlaceTool | ScatterTool | ModuleTool
            | TurnClockwise | TurnCounter | NavmeshOverlay | ContoursOverlay
            | FootprintsOverlay | BrushSmaller | BrushLarger | FrameSelection | TopDown
            | SelectAtCursor | PasteInPlace | MakeBlueprint | Listen => KeyScope::Area,
            FilterScene | OpenBlueprint | Group | Ungroup | NewFolder | Reorder | Cut | Hide
            | Lock | ShowOnly | ShowEverything | UnlockEverything => KeyScope::SceneGraph,
            Shrink | Grow | Pin | Route => KeyScope::WorldMap,
            PlayPause | PreviousFrame | NextFrame | Mute | ApexCue | SoundCue | StepCue
            | InsertFrame | FrameShorter | FrameLonger | FrameEarlier | FrameLater => {
                KeyScope::Model
            }
            RunScript | Palette | Outline | AddChoice | Link => KeyScope::Script,
            NextCell | FillDown => KeyScope::Spreadsheet,
        }
    }

    pub fn about(self) -> &'static str {
        use EditorKey::*;
        match self {
            Search => "search rows, keys and files",
            Save => "save every unsaved file",
            Undo => "undo",
            Redo => "redo",
            CloseTab => "close the tab",
            Quit => "quit",
            KeySheet => "this sheet",
            FrameRate => "show or hide the frame rate",
            PlayFromStart => "play from its own start",
            PlayHere => "play here",
            StopPlay => "stop playing",
            PlayedCharacter => "open the test character being played",
            Rename => "rename the selected file, folder or node",
            NewAgain => "make the type made last again, in the content tree's folder",
            CopyPath => "copy the selected file's path",
            Usages => "show usages",
            Duplicate => "duplicate the selection",
            Delete => "delete the selection, after showing usages",
            Confirm => "open, edit or place the selection",
            NewLine => "a line break in a text",
            OpenInNewTab => "open in a new tab",
            Cancel => "close a picker, a menu or a preview",
            Move => "move, or nudge the selection",
            Pick => "pick a numbered entry",
            Copy => "copy",
            Paste => "paste at the cursor",
            SelectAll => "select everything",
            ToggleSelection => "held while clicking: add or remove one",
            ExtendSelection => "held while clicking: select up to it",
            FineStep => "held: the finest step",
            Unsnapped => "held: a finer snap, or any point",
            Pan => "held while dragging: pan",
            Zoom => "held while scrolling: zoom",
            ContextMenu => "the menu at the cursor",
            FirstLine => "the first line of a list",
            LastLine => "the last line of a list",
            SelectTool => "select",
            SculptTool => "sculpt the ground",
            PaintTool => "paint the ground",
            PlaceTool => "place a prop or blueprint",
            ScatterTool => "scatter props",
            ModuleTool => "module",
            TurnClockwise => "turn 15° clockwise",
            TurnCounter => "turn 15° counterclockwise",
            NavmeshOverlay => "navmesh overlay",
            ContoursOverlay => "height contours",
            FootprintsOverlay => "footprints overlay",
            BrushSmaller => "a smaller brush",
            BrushLarger => "a larger brush",
            FrameSelection => "frame the selection",
            TopDown => "look straight down",
            SelectAtCursor => "select what is under the keyboard cursor",
            PasteInPlace => "paste at the same spot",
            MakeBlueprint => "make a blueprint of the selection",
            Listen => "held while clicking: listen at a spot",
            FilterScene => "filter the scene",
            OpenBlueprint => "open a placement's blueprint",
            Group => "group into a new folder",
            Ungroup => "ungroup",
            NewFolder => "new folder here",
            Reorder => "move up or down among its siblings",
            Cut => "cut, to paste into the selected folder",
            Hide => "hide or show",
            Lock => "lock or unlock",
            ShowOnly => "held clicking an eye: show only this; again to restore",
            ShowEverything => "show everything",
            UnlockEverything => "unlock everything",
            Shrink => "selected cards 10% smaller",
            Grow => "selected cards 10% bigger",
            Pin => "pin · unpin",
            Route => "held clicking two areas: the route",
            PlayPause => "play or pause the strip",
            PreviousFrame => "step a frame back",
            NextFrame => "step a frame on",
            Mute => "mute the preview",
            ApexCue => "apex cue on the frame",
            SoundCue => "sound cue on the frame",
            StepCue => "step cue on the frame",
            InsertFrame => "insert a frame",
            FrameShorter => "frame 10 ms shorter",
            FrameLonger => "frame 10 ms longer",
            FrameEarlier => "move the frame earlier",
            FrameLater => "move the frame later",
            RunScript => "run the script",
            Palette => "the palette: a node after the selection",
            Outline => "canvas · outline",
            AddChoice => "add a choice",
            Link => "link an output to a node by name",
            NextCell => "the next cell",
            FillDown => "fill down",
        }
    }

    fn binding(self) -> Binding {
        use EditorKey::*;
        let plain = Modifiers::NONE;
        let command = Modifiers::COMMAND;
        let press = |modifiers: Modifiers, key: Key| {
            Binding::Press(vec![KeyboardShortcut::new(modifiers, key)])
        };
        match self {
            Search => press(command, Key::P),
            Save => press(command, Key::S),
            Undo => press(command, Key::Z),
            Redo => press(command | Modifiers::SHIFT, Key::Z),
            CloseTab => press(command, Key::W),
            Quit => press(command, Key::Q),
            KeySheet => press(plain, Key::F1),
            FrameRate => press(plain, Key::F3),
            PlayFromStart => press(plain, Key::F5),
            PlayHere => press(plain, Key::F6),
            StopPlay => press(Modifiers::SHIFT, Key::F5),
            PlayedCharacter => press(command, Key::T),
            Rename => press(plain, Key::F2),
            NewAgain => press(command, Key::N),
            CopyPath => press(command | Modifiers::SHIFT, Key::C),
            Usages => press(Modifiers::SHIFT, Key::F12),
            Duplicate | FillDown => press(command, Key::D),
            Delete => Binding::Press(vec![
                KeyboardShortcut::new(plain, Key::Delete),
                KeyboardShortcut::new(plain, Key::Backspace),
            ]),
            Confirm => Binding::Press(vec![ui::keys::CONFIRM]),
            NewLine => press(Modifiers::SHIFT, Key::Enter),
            OpenInNewTab | AddChoice | OpenBlueprint => press(command, Key::Enter),
            Cancel => Binding::Press(vec![ui::keys::CANCEL]),
            Move => Binding::Arrows(plain),
            Pick => Binding::Digits,
            Copy => press(command, Key::C),
            Paste => press(command, Key::V),
            SelectAll => press(command, Key::A),
            ToggleSelection | Zoom => Binding::Hold(command),
            ExtendSelection | FineStep => Binding::Hold(Modifiers::SHIFT),
            Unsnapped | Listen | Route => Binding::Hold(Modifiers::ALT),
            Pan => Binding::HoldKey(Key::Space),
            ContextMenu => press(Modifiers::SHIFT, Key::F10),
            FirstLine => press(plain, Key::Home),
            LastLine => press(plain, Key::End),
            SelectTool => press(plain, Key::V),
            SculptTool => press(plain, Key::S),
            PaintTool => press(plain, Key::B),
            PlaceTool => press(plain, Key::P),
            ScatterTool => press(plain, Key::G),
            ModuleTool => press(plain, Key::M),
            TurnClockwise => press(plain, Key::R),
            TurnCounter => press(Modifiers::SHIFT, Key::R),
            NavmeshOverlay => press(plain, Key::N),
            ContoursOverlay => press(plain, Key::H),
            FootprintsOverlay => press(Modifiers::SHIFT, Key::P),
            BrushSmaller => press(plain, Key::OpenBracket),
            BrushLarger => press(plain, Key::CloseBracket),
            FrameSelection => press(plain, Key::F),
            TopDown => press(plain, Key::Num7),
            FilterScene => press(command, Key::F),
            Group => press(command, Key::G),
            Ungroup => press(command | Modifiers::SHIFT, Key::G),
            NewFolder => press(command | Modifiers::SHIFT, Key::N),
            Reorder => Binding::Arrows(command),
            Cut => press(command, Key::X),
            Hide => press(command | Modifiers::SHIFT, Key::H),
            Lock => press(command | Modifiers::SHIFT, Key::L),
            ShowOnly => Binding::Hold(Modifiers::ALT),
            ShowEverything => press(Modifiers::ALT | Modifiers::SHIFT, Key::H),
            UnlockEverything => press(Modifiers::ALT | Modifiers::SHIFT, Key::L),
            SelectAtCursor | PlayPause => press(plain, Key::Space),
            PasteInPlace => press(command | Modifiers::SHIFT, Key::V),
            MakeBlueprint => press(command | Modifiers::ALT, Key::B),
            Shrink => press(plain, Key::Minus),
            Grow => Binding::Press(vec![
                KeyboardShortcut::new(plain, Key::Equals),
                KeyboardShortcut::new(plain, Key::Plus),
            ]),
            Pin => press(plain, Key::P),
            PreviousFrame => press(plain, Key::Comma),
            NextFrame => press(plain, Key::Period),
            Mute => press(plain, Key::M),
            ApexCue => press(plain, Key::A),
            SoundCue => press(plain, Key::S),
            StepCue => press(plain, Key::T),
            InsertFrame => press(plain, Key::Insert),
            FrameShorter => press(Modifiers::ALT, Key::ArrowLeft),
            FrameLonger => press(Modifiers::ALT, Key::ArrowRight),
            FrameEarlier => press(command, Key::ArrowLeft),
            FrameLater => press(command, Key::ArrowRight),
            RunScript => press(plain, Key::F8),
            Palette | NextCell => press(plain, Key::Tab),
            Outline => press(plain, Key::O),
            Link => press(command, Key::L),
        }
    }
}

enum Binding {
    Press(Vec<KeyboardShortcut>),
    Arrows(Modifiers),
    Digits,
    Hold(Modifiers),
    HoldKey(Key),
}

const DIGITS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

const ARROWS: [(Key, [i32; 2]); 4] = [
    (Key::ArrowLeft, [-1, 0]),
    (Key::ArrowRight, [1, 0]),
    (Key::ArrowUp, [0, -1]),
    (Key::ArrowDown, [0, 1]),
];

fn claims(chord: &KeyboardShortcut, input: &InputState) -> bool {
    matches(chord, input) && !outranked(chord, input)
}

fn matches(chord: &KeyboardShortcut, input: &InputState) -> bool {
    input.key_pressed(chord.logical_key) && input.modifiers.matches_logically(chord.modifiers)
}

fn outranked(chord: &KeyboardShortcut, input: &InputState) -> bool {
    EditorKey::VARIANTS
        .iter()
        .flat_map(|key| match key.binding() {
            Binding::Press(chords) => chords,
            Binding::Arrows(modifiers) => ARROWS
                .iter()
                .map(|(arrow, _)| KeyboardShortcut::new(modifiers, *arrow))
                .collect(),
            _ => Vec::new(),
        })
        .any(|other| {
            other.logical_key == chord.logical_key
                && other.modifiers != chord.modifiers
                && other.modifiers.contains(chord.modifiers)
                && matches(&other, input)
        })
}

fn arrow(input: &InputState, modifiers: Modifiers) -> Option<[i32; 2]> {
    ARROWS.iter().find_map(|(key, step)| {
        claims(&KeyboardShortcut::new(modifiers, *key), input).then_some(*step)
    })
}

fn digit(input: &InputState) -> Option<usize> {
    DIGITS
        .iter()
        .position(|key| claims(&KeyboardShortcut::new(Modifiers::NONE, *key), input))
}

fn key_name(key: Key) -> &'static str {
    match key {
        Key::Escape => "Esc",
        Key::Delete => "Del",
        Key::Insert => "Ins",
        Key::ArrowLeft => "←",
        Key::ArrowRight => "→",
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        other => other.symbol_or_name(),
    }
}
