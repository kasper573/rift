//! What the editor's probe speaks: the questions a driver asks a running editor, each answered by
//! a frame after it with one JSON line, and the answers. Window positions are the window's logical
//! pixels; ground positions are metres on the ground of the area the stage shows.

use bevy::math::{Mat4, Vec4};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ProbeInput, ProbeUiElement};
use crate::core::math::{MatrixSchema, Offset, Pos, PosSchema, Rect, RectSchema, Size, SizeSchema};
use crate::core::units::{Meters, Radians, WindowPx};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[serde(tag = "ask", rename_all = "snake_case")]
pub enum EditorFrameQuestion {
    /// Answered with an [`EditorSnapshot`] of a frame after the question.
    Snapshot,
    /// The frames drawn after frame `since`, as far as they have settled. Answered with
    /// [`super::ProbeFrames`].
    Frames { since: u32 },
    /// Answered with the [`EditorTrees`] a frame after the question drew.
    TreeRows,
}

/// One frame of the editor.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct EditorSnapshot {
    pub frame: u32,
    pub input: ProbeInput,
    #[schemars(with = "Option<SizeSchema>")]
    pub screen: Option<Size<WindowPx>>,
    /// The graphics adapter the editor draws with, by the name its driver gives it.
    pub adapter: Option<String>,
    /// What the interface shows: text, icons, fields and sliders, less what is scrolled or clipped
    /// out of sight.
    pub ui: Vec<ProbeUiElement>,
    /// Where the interface takes the pointer: each rect edges included, as egui hit-tests.
    #[schemars(with = "Vec<RectSchema>")]
    pub covered: Vec<Rect<WindowPx>>,
    /// Whether a menu is open over the interface.
    pub menu_open: bool,
    /// The panels the window is parted into, each resized by dragging its inner edge.
    pub panels: Vec<EditorPanel>,
    /// The title of the tab in front.
    pub tab: Option<String>,
    /// The stage of the area or blueprint view in front, when one shows.
    pub stage: Option<EditorStage>,
    pub scenery: EditorScenery,
    /// Whether the editor is busy with work the interface waits on, such as opening a folder.
    pub busy: bool,
    /// The rows selected, as the scene tree names them.
    pub selected: Vec<String>,
    /// How many edits undo and redo would step through.
    pub undoable: usize,
    pub redoable: usize,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct EditorPanel {
    pub name: String,
    #[schemars(with = "RectSchema")]
    pub rect: Rect<WindowPx>,
}

/// The stage of an area or a blueprint: where it shows in the window, how its camera projects the
/// ground into it, and where what the stage holds shows.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct EditorStage {
    #[schemars(with = "RectSchema")]
    pub viewport: Rect<WindowPx>,
    #[schemars(with = "MatrixSchema")]
    pub view_proj: Mat4,
    #[schemars(with = "PosSchema")]
    pub target: Pos<Meters>,
    pub yaw: Radians,
    pub pitch: Radians,
    pub distance: Meters,
    /// The farthest the camera may draw back.
    pub farthest: Meters,
    /// What the stage holds nearest the middle of its viewport, each where it shows.
    pub marks: Vec<EditorStageMark>,
    /// The row of the scene tree a click at the pointer picks, when the pointer is over the stage.
    pub picked: Option<String>,
}

impl EditorStage {
    /// Where a point `height` above the ground at `at` shows in the window.
    pub fn window_point(&self, at: Pos<Meters>, height: Meters) -> Pos<WindowPx> {
        let clip = self.view_proj * Vec4::new(at.x, height.0, at.y, 1.0);
        let ndc = clip.truncate() / clip.w;
        self.viewport.origin
            + Offset::new(
                (ndc.x + 1.0) / 2.0 * self.viewport.size.width,
                (1.0 - ndc.y) / 2.0 * self.viewport.size.height,
            )
    }
}

/// Something the stage holds, by its row of the scene tree, where its middle shows in the window.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct EditorStageMark {
    pub row: String,
    #[schemars(with = "PosSchema")]
    pub at: Pos<WindowPx>,
}

/// How the scenery of the views stands.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EditorScenery {
    /// No view draws an area.
    None,
    /// What a view takes in is still being stood or readied.
    Standing,
    /// Every view's scenery stands whole.
    Stood,
}

/// The trees the editor drew in a frame, each with the rows it showed.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct EditorTrees {
    pub trees: Vec<EditorTree>,
}

/// A tree as drawn: the game's files or an area's scene, by `name`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct EditorTree {
    pub name: String,
    /// Where the tree shows its rows, scrolled.
    #[schemars(with = "RectSchema")]
    pub rect: Rect<WindowPx>,
    /// The rows it drew, top to bottom, those scrolled out of the tree's rect included.
    pub rows: Vec<EditorTreeRow>,
    pub selected: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct EditorTreeRow {
    pub id: String,
    #[schemars(with = "RectSchema")]
    pub rect: Rect<WindowPx>,
    pub folder: bool,
    pub open: bool,
}
