//! Baking the game's maps from the editor: when the designer asks, from the File menu, and on its
//! own for every area whose map was never baked, as soon as the game the editor holds has one, so
//! no area plays without a map. A bake is one step of the History, saved with the rest, and the
//! editor waits for it as it waits for any work.

use std::collections::HashSet;
use std::sync::Arc;

use bevy::prelude::World;
use engine::core::content::Content;
use engine::systems::area::{AreaDef, AreaId};
use engine::systems::area_map::bake::{self, BakedMaps, MapBakeProgress};

use crate::app::Editor;
use crate::busy::Progress;
use crate::project::{ContentEdit, FileBytes};

/// The bakes the editor asks for and runs.
#[derive(Default)]
pub struct MapBakes {
    asked: bool,
    running: Option<Running>,
    /// The areas a bake was started for on its own, by path, so one whose map doesn't bake is
    /// never baked again and again.
    forced: HashSet<String>,
}

/// Asks for the maps of every area of the open game to be baked: the squares whose ground, props
/// or style changed since they were baked are drawn again.
pub fn bake_all(editor: &mut Editor) {
    editor.maps.asked = true;
}

/// Whether a bake is asked for or under way.
pub fn baking(editor: &Editor) -> bool {
    editor.maps.asked || editor.maps.running.is_some()
}

/// Moves the bakes on by a frame: starts the one asked for, or one of the areas never baked, once
/// the editor holds the game as its files stand; tells how far the one under way has come; and
/// writes what it baked once it is done.
pub(crate) fn drive(editor: &mut Editor, world: &mut World) {
    if let Some(running) = &editor.maps.running {
        if let Some(progress) = bake::progress(world) {
            running.tell(progress);
            return;
        }
        if let Some(baked) = bake::take_baked(world) {
            let running = editor.maps.running.take().expect("a bake under way");
            write(editor, &running.content, &baked);
        }
        return;
    }
    if editor.busy.is_busy() || editor.play.running.is_some() {
        return;
    }
    let Some(content) = settled_content(editor) else {
        return;
    };
    let areas: Vec<AreaId> = if editor.maps.asked {
        editor.maps.asked = false;
        content.ids::<AreaDef>().collect()
    } else {
        content
            .ids::<AreaDef>()
            .filter(|area| {
                content
                    .table::<AreaDef>()
                    .get(*area)
                    .is_some_and(|def| def.map.is_empty())
                    && editor.maps.forced.insert(area.path(&content).to_owned())
            })
            .collect()
    };
    if areas.is_empty() {
        return;
    }
    let progress = Arc::new(Progress::default());
    progress.doing(PLANNING, None);
    bake::begin(world, content.clone(), areas);
    editor.maps.running = Some(Running {
        content,
        progress: progress.clone(),
    });
    editor.wait_on("Baking the maps", progress, |editor| {
        editor.maps.running.is_none()
    });
}

/// A bake under way, of the game as `content` holds it.
struct Running {
    content: Content,
    progress: Arc<Progress>,
}

impl Running {
    fn tell(&self, progress: MapBakeProgress) {
        let (doing, done, total) = match progress {
            MapBakeProgress::Planning { come, total } => (PLANNING, come, total),
            MapBakeProgress::Drawing { drawn, total } => (DRAWING, drawn, total),
        };
        if self.progress.is_doing(doing) {
            self.progress.done(done as usize);
        } else {
            self.progress.doing(doing, Some(total as usize));
        }
    }
}

const PLANNING: &str = "Finding the props' plans from above";
const DRAWING: &str = "Drawing the squares";

/// The game the editor holds, once it holds it as its files stand.
fn settled_content(editor: &Editor) -> Option<Content> {
    let project = editor.project.as_ref()?;
    if project.loading() || project.read_only.is_some() {
        return None;
    }
    project.content.clone()
}

/// Writes the maps `baked` of the areas `content` names as one step of the History: only the
/// files of each that changed.
fn write(editor: &mut Editor, content: &Content, baked: &BakedMaps) {
    let Some(reader) = editor.reader() else {
        return;
    };
    let mut edit = ContentEdit::new("Bake the maps");
    for area in &baked.areas {
        let path = area.area.path(content);
        let Some(Ok(row)) = editor.read::<AreaDef>(path) else {
            continue;
        };
        let mut row = (*row).clone();
        row.map = area.map.clone();
        match reader.write_only(path, &row, area.changes(path)) {
            Ok(written) => {
                for (file, bytes) in written.files {
                    let held = editor
                        .project
                        .as_ref()
                        .and_then(|project| project.files.get(&file));
                    if held.as_deref() != Some(bytes.as_slice()) {
                        edit.write(file, FileBytes::from(bytes));
                    }
                }
                for file in written.removed {
                    edit.remove(file);
                }
            }
            Err(error) => {
                editor.toast(format!(
                    "The map of {} does not write: {error}",
                    crate::tabs::shown_path(path)
                ));
                return;
            }
        }
    }
    let drawn: usize = baked.areas.iter().map(|area| area.drawn()).sum();
    editor.say(format!(
        "Baked the maps of {} areas in {:.1?}, drawing {drawn} squares again",
        baked.areas.len(),
        baked.took
    ));
    if !edit.changes.is_empty() {
        editor.apply(edit);
    }
}
