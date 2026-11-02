use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use engine::core::time::UnixMillis;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use strum::VariantArray;

use crate::views::area::camera::OrbitCamera;

// Per-user state stays outside content folders, so it never ships with a game.
#[derive(Clone, Debug)]
pub struct UserFolder {
    root: PathBuf,
}

impl UserFolder {
    pub fn at(root: PathBuf) -> UserFolder {
        UserFolder { root }
    }

    pub fn locate() -> UserFolder {
        let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
        UserFolder::at(base.join("rift-editor"))
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub(crate) fn journal_dir(&self) -> PathBuf {
        self.root.join("journal")
    }

    pub(crate) fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    fn layouts_dir(&self) -> PathBuf {
        self.root.join("layouts")
    }

    pub(crate) fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }

    pub(crate) fn load_settings(&self) -> EditorSettings {
        std::fs::read_to_string(self.settings_file())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save_settings(&self, settings: &EditorSettings) {
        let _ = std::fs::create_dir_all(&self.root);
        if let Ok(text) = serde_json::to_string_pretty(settings) {
            let _ = std::fs::write(self.settings_file(), text + "\n");
        }
    }

    pub fn folder_key(folder: &Path) -> String {
        blake3::hash(folder.to_string_lossy().as_bytes()).to_hex()[..16].to_owned()
    }

    pub(crate) fn load_layout<T: DeserializeOwned + Default>(
        &self,
        project: &Path,
        slug: &str,
    ) -> T {
        std::fs::read_to_string(self.layout_file(project, slug))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save_layout<T: Serialize>(
        &self,
        project: &Path,
        slug: &str,
        layout: &T,
    ) -> Result<PathBuf, std::io::Error> {
        let file = self.layout_file(project, slug);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(layout).map_err(std::io::Error::other)?;
        std::fs::write(&file, text + "\n")?;
        Ok(file)
    }

    fn layout_file(&self, project: &Path, slug: &str) -> PathBuf {
        self.layouts_dir()
            .join(UserFolder::folder_key(project))
            .join(format!("{slug}.json"))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EditorSettings {
    pub recent: Vec<PathBuf>,
    pub recent_at: BTreeMap<String, UnixMillis>,
    pub binaries: Option<PathBuf>,
    pub stamps: BTreeMap<String, StampSettings>,
    pub cameras: BTreeMap<String, OrbitCamera>,
    pub listeners: BTreeMap<String, [f32; 2]>,
    pub views: BTreeMap<String, SpreadsheetView>,
    pub last_play: Option<String>,
    /// By project folder, how its content tree is shown.
    pub content_trees: BTreeMap<String, TreeView>,
    /// By project folder and bundle, how a scene graph is shown.
    pub scene_trees: BTreeMap<String, TreeView>,
    /// Whether the area and blueprint boxes above a scene graph are folded to one line.
    pub boxes_folded: bool,
    /// Whether the meter of frames drawn a second shows.
    pub frame_rate: bool,
}

/// How a tree is shown: the folders open, by path, and the kinds of rows left out.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TreeView {
    pub open: std::collections::BTreeSet<String>,
    pub hidden_kinds: std::collections::BTreeSet<String>,
    pub hide_empty_folders: bool,
}

impl TreeView {
    /// Keeps open the folders that moved, where they went.
    pub fn moved(&mut self, moved: &dyn Fn(&str) -> Option<String>) {
        self.open = std::mem::take(&mut self.open)
            .into_iter()
            .map(|path| moved(&path).unwrap_or(path))
            .collect();
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StampSettings {
    /// How many quarter turns clockwise each placement is turned.
    pub quarter_turns: u8,
    pub spacing: Option<u32>,
    /// The blueprint's variables as last set, as their files write them: they name files, so they
    /// read only against the game as it stands when the stamp places, not as settings load.
    pub values: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SpreadsheetView {
    pub columns: Vec<String>,
    pub sort: Option<(String, bool)>,
    pub filters: Vec<SheetFilter>,
}

/// A test a sheet's rows pass to show.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SheetFilter {
    /// An npc some script lets a player fight.
    CanBeFought,
    /// A row holding a module of the tag.
    Has(String),
    /// A row holding no module of the tag.
    Lacks(String),
    /// A row whose number in the column compares to the value as the test says.
    Compare {
        column: String,
        test: Comparison,
        value: f64,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, VariantArray)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    Below,
    AtMost,
    Equal,
    AtLeast,
    Above,
}

impl Comparison {
    pub fn holds(self, have: f64, want: f64) -> bool {
        match self {
            Comparison::Below => have < want,
            Comparison::AtMost => have <= want,
            Comparison::Equal => (have - want).abs() < f64::EPSILON,
            Comparison::AtLeast => have >= want,
            Comparison::Above => have > want,
        }
    }

    pub fn sign(self) -> &'static str {
        match self {
            Comparison::Below => "<",
            Comparison::AtMost => "≤",
            Comparison::Equal => "=",
            Comparison::AtLeast => "≥",
            Comparison::Above => ">",
        }
    }
}

impl SheetFilter {
    /// The filter as its chip reads.
    pub fn label(&self) -> String {
        match self {
            SheetFilter::CanBeFought => "can be fought".to_owned(),
            SheetFilter::Has(tag) => format!("has {tag}"),
            SheetFilter::Lacks(tag) => format!("lacks {tag}"),
            SheetFilter::Compare {
                column,
                test,
                value,
            } => format!("{column} {} {value}", test.sign()),
        }
    }
}

impl EditorSettings {
    pub fn remember(&mut self, folder: &Path) {
        self.recent.retain(|known| known != folder);
        self.recent.insert(0, folder.to_path_buf());
        self.recent.truncate(12);
        self.recent_at
            .insert(folder.display().to_string(), SystemTime::now().into());
        self.recent_at.retain(|path, _| {
            self.recent
                .iter()
                .any(|known| known.display().to_string() == *path)
        });
    }

    pub fn forget(&mut self, folder: &Path) {
        self.recent.retain(|known| known != folder);
        self.recent_at.remove(&folder.display().to_string());
    }

    pub(crate) fn opened_ago(&self, folder: &Path) -> String {
        let Some(at) = self.recent_at.get(&folder.display().to_string()) else {
            return String::new();
        };
        ui::words::ago(UnixMillis::from(SystemTime::now()).since(*at).0)
    }
}
