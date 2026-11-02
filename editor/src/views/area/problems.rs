//! What the loader finds wrong with an area, where the designer can act on it: counted in the
//! status bar, and listed in the Output tab with a way to go there and, when the problem knows
//! where its module would be fine, to move it there.

use engine::core::content::{ContentFile, ErrorPlace, LoadError};
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::AreaDef;
use engine::systems::area::scene::scene_file;
use ui::controls;
use ui::egui::Ui;
use ui::face;
use ui::theme;

use super::nodes;
use crate::app::Editor;
use crate::project::RowSelection;
use crate::project::files::bundle_of;
use crate::scene_graph;
use crate::tabs::EditorTab;

/// A problem the loader found, placed for the editor: the file and the scene node it is on, and the
/// spot it suggests when it names one.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub file: String,
    pub pointer: String,
    pub text: String,
    pub node: Option<String>,
    pub suggested: Option<Pos<Meters>>,
}

impl Problem {
    pub fn of(error: &LoadError) -> Problem {
        let pointer = match &error.at {
            ErrorPlace::Json { pointer, .. } => pointer.clone(),
            _ => String::new(),
        };
        let node = scene_graph::node_pointer(&pointer);
        Problem {
            file: error.file.clone(),
            pointer,
            text: error.problem.to_string(),
            node: (!node.is_empty() && error.file.ends_with(&format!("/{}", scene_name())))
                .then_some(node),
            suggested: error.problem.spot,
        }
    }

    /// The bundle the problem is in, when it is in an area's or a blueprint's files.
    pub fn bundle(&self) -> Option<&str> {
        bundle_of(&self.file)
            .map(|(bundle, _)| bundle)
            .filter(|bundle| {
                engine::core::content::table_of(bundle).is_some_and(|table| {
                    table == AreaDef::TABLE
                        || table == engine::systems::area::blueprint::BlueprintDef::TABLE
                })
            })
    }
}

fn scene_name() -> String {
    scene_file("").trim_start_matches('/').to_owned()
}

/// Every problem the last load found.
pub fn all(editor: &Editor) -> Vec<Problem> {
    editor
        .project
        .as_ref()
        .and_then(|project| project.report.as_ref())
        .map(|report| report.errors.iter().map(Problem::of).collect())
        .unwrap_or_default()
}

/// The problems in `bundle`'s files.
pub(crate) fn in_bundle(editor: &Editor, bundle: &str) -> Vec<Problem> {
    all(editor)
        .into_iter()
        .filter(|problem| problem.bundle() == Some(bundle))
        .collect()
}

/// Opens where the problem is: the bundle's view with its node selected, or the row's own tab.
pub(crate) fn go_to(editor: &mut Editor, problem: &Problem) {
    if let Some(bundle) = problem.bundle() {
        let at = problem
            .node
            .as_ref()
            .and_then(|node| {
                nodes::module_node(&nodes::scene_of(editor, bundle), node)
                    .and_then(|module| super::shapes::bounds(&module.module, None))
            })
            .map(|bounds| {
                let center = bounds.center();
                Pos::new(center.x, center.y)
            });
        if engine::core::content::table_of(bundle) == Some(AreaDef::TABLE) {
            super::open_area_at(editor, bundle, at, problem.node.clone());
        } else {
            editor.open_tab(EditorTab::Blueprint {
                key: bundle.to_owned(),
            });
            if let Some(node) = &problem.node {
                editor.selection = RowSelection::parts(
                    engine::systems::area::blueprint::BlueprintDef::TABLE,
                    bundle,
                    vec![node.clone()],
                );
            }
        }
        return;
    }
    if let Some((table, key)) = crate::project::files::content_of(&problem.file) {
        let (table, key) = (table.to_owned(), key.to_owned());
        editor.open_row(&table, &key);
    }
}

/// The problems of the whole folder, each with a way to go there, at the top of the Output tab.
pub fn output(editor: &mut Editor, ui: &mut Ui) {
    let problems = all(editor);
    if problems.is_empty() {
        return;
    }
    ui.label(theme::warn(format!(
        "{} problem{}",
        problems.len(),
        ui::words::plural(problems.len())
    )));
    let mut go = None;
    let mut fix = None;
    for problem in problems.iter().take(SHOWN) {
        ui.horizontal(|ui| {
            if controls::button(ui, "Go to").clicked() {
                go = Some(problem.clone());
            }
            if let Some((bundle, node, to)) = movable(problem)
                && controls::button(ui, "Move there").clicked()
            {
                fix = Some((bundle.to_owned(), node.to_owned(), to));
            }
            ui.label(
                face::mono(format!(
                    "{}  {}",
                    crate::project::problem_place(&problem.file, &problem.pointer),
                    problem.text
                ))
                .size(12.5),
            );
        });
    }
    if problems.len() > SHOWN {
        ui.label(theme::muted(format!("and {} more", problems.len() - SHOWN)).size(12.5));
    }
    ui.add_space(6.0);
    if let Some(problem) = go {
        go_to(editor, &problem);
    }
    if let Some((bundle, node, to)) = fix {
        move_there(editor, &bundle, &node, to);
    }
}

/// Moves the module at `node` of `bundle` to where its problem says it would be fine.
pub fn move_there(editor: &mut Editor, bundle: &str, node: &str, to: Pos<Meters>) {
    let reader = editor.reader();
    nodes::change_scene(editor, bundle, |scene| {
        let module = &mut nodes::module_node_mut(scene, node)?.module;
        let from = super::shapes::bounds(module, None)?.center();
        module.shift(to.x - from.x, to.y - from.y);
        Some(format!(
            "Move {} to {:.2}, {:.2}",
            reader
                .as_ref()
                .map(|reader| super::shapes::label(module, reader.as_ref()))
                .unwrap_or_default(),
            to.x,
            to.y
        ))
    });
}

/// The module a problem is on and where it suggests moving it, when it does.
fn movable(problem: &Problem) -> Option<(&str, &str, Pos<Meters>)> {
    Some((
        problem.bundle()?,
        problem.node.as_deref()?,
        problem.suggested?,
    ))
}

const SHOWN: usize = 12;
