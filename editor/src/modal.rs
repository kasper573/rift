use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::AreaModuleKind;
use serde_json::Value;

use crate::project::FileConflict;
use crate::rows;
use crate::views::area::ResizeDialog;
use crate::views::script::{LinkNodeDialog, RenameNodeDialog, ScriptRow};
use crate::views::world::ArrangeDialog;

pub enum ModalDialog {
    NewGame(NewGameDialog),
    Confirm(ConfirmDialog),
    Conflicts(Vec<FileConflict>),
    Delete(DeleteDialog),
    FindUnused(FindUnusedDialog),
    Close(CloseDialog),
    Binaries,
    Keys,
    Video,
    Resize(ResizeDialog),
    Arrange(ArrangeDialog),
    PickRow(PickRowDialog),
    RenameNode(RenameNodeDialog),
    LinkNode(LinkNodeDialog),
    Import(crate::import::ImportDialog),
}

pub struct PickRowDialog {
    pub title: String,
    pub table: String,
    pub search: String,
    pub picked: Option<String>,
    pub options: Vec<String>,
    pub then: PickRowThen,
}

pub enum PickRowThen {
    /// Adds a module of `kind` at `at` to `area`, naming the picked file.
    AddModule {
        area: String,
        kind: AreaModuleKind,
        at: Option<Pos<Meters>>,
    },
    ReplaceBlueprint {
        area: String,
        node: String,
    },
    NewFile {
        folder: String,
    },
    MoveInto {
        paths: Vec<String>,
    },
    /// Moves rows of a scene graph into the folder picked, by its labels.
    MoveNodes {
        bundle: String,
        ids: Vec<String>,
        folders: Vec<(String, String)>,
    },
}

impl PickRowThen {
    /// The file the picked row is written into, when it is one.
    pub(crate) fn editing(&self) -> Option<&str> {
        match self {
            PickRowThen::AddModule { area, .. } | PickRowThen::ReplaceBlueprint { area, .. } => {
                Some(area)
            }
            _ => None,
        }
    }
}

pub struct NewGameDialog {
    pub source: NewGameSource,
    pub name: String,
    pub folder: String,
    pub folder_touched: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
pub enum NewGameSource {
    Template(String),
    Copy {
        folder: PathBuf,
        /// How many files the folder holds, once they are counted in the background.
        files: Arc<OnceLock<usize>>,
    },
}

impl NewGameSource {
    /// A copy of the game at `folder`, its files counted in the background meanwhile.
    pub fn copy(folder: PathBuf) -> NewGameSource {
        let files = Arc::new(OnceLock::new());
        let (counted, from) = (files.clone(), folder.clone());
        std::thread::spawn(move || {
            let listed = crate::project::files::list(&from, &|_| {});
            let _ = counted.set(listed.map_or(0, |listed| listed.len()));
        });
        NewGameSource::Copy { folder, files }
    }
}

pub struct ConfirmDialog {
    pub title: String,
    pub body: String,
    pub label: String,
    pub compare: Option<TextComparison>,
    pub action: ConfirmAction,
}

pub struct TextComparison {
    pub left_title: String,
    pub left: String,
    pub right_title: String,
    pub right: String,
}

pub enum ConfirmAction {
    ReplaceModule {
        table: String,
        key: String,
        index: usize,
        module: Value,
    },
    DeleteScriptNodes {
        script: ScriptRow,
        names: Vec<String>,
    },
}

impl ConfirmAction {
    pub(crate) fn destroys(&self) -> bool {
        match self {
            ConfirmAction::ReplaceModule { .. } => false,
            ConfirmAction::DeleteScriptNodes { .. } => true,
        }
    }
}

pub struct DeleteDialog {
    /// The files and folders picked to delete.
    pub paths: Vec<String>,
    /// Every content file and media file that goes with them.
    pub gone: Vec<String>,
    pub groups: Vec<rows::DeletedUsages>,
    pub error: Option<String>,
}

pub struct FindUnusedDialog {
    /// What is left out of the deletion; everything else found is in it.
    pub unticked: BTreeSet<crate::unused::UnusedItem>,
    pub open: BTreeSet<String>,
    pub error: Option<String>,
}

pub struct CloseDialog {
    pub then: CloseThen,
}

#[derive(Clone)]
pub enum CloseThen {
    Quit,
    Close,
    Open(PathBuf),
}
