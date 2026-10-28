//! The sets page: what anyone can ask of the skeleton (its engine actions, poses, every move a set
//! names, and the gestures of a line's expressions and every other one a set names) against base
//! and each stance it has a set for, for the build picked. A cell a set of the build names is
//! plain, one the ladder finds further down italic. A cell clicked opens below the table: its clips,
//! picked from every clip that plays by any part of its file's path, and the clip chosen there on
//! the stage, its cues timed on its timeline and, for a gait, its pace. Every build but the first
//! fills its empty cells from the first's by rewriting their files' paths.

use engine::core::content::{Content, NamedContent};
use engine::core::units::{Meters, MetersPerSec};
use engine::systems::actor::skeleton::{
    BuildName, BuildSets, Clip, ClipCell, ClipCues, ClipOrder, ClipPace, ClipRef, ClipSet,
    GestureName, GroupName, MoveName, Rung, SkeletonDef, StanceSet,
};
use engine::systems::actor::stance::{StanceDef, StanceId};
use engine::systems::actor::{Action, Motion, Pose};
use engine::systems::dialogue::bust::GenericExpression;
use strum::VariantArray;
use ui::egui::{self, RichText, Ui, Vec2};
use ui::{chip, controls, form, panel, theme, words};

use super::rigging::Rigging;
use crate::app::Editor;
use crate::views::cues;

/// A row of the table: what is asked of the skeleton.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetRow {
    Action(Action),
    Pose(Pose),
    Begin(Pose),
    End(Pose),
    Move(MoveName),
    Gesture(GestureName),
}

impl SetRow {
    pub fn label(&self) -> String {
        match self {
            SetRow::Action(action) => action.as_ref().to_owned(),
            SetRow::Pose(pose) => format!("{} · pose", pose.as_ref()),
            SetRow::Begin(pose) => format!("{} · taking it", pose.as_ref()),
            SetRow::End(pose) => format!("{} · leaving it", pose.as_ref()),
            SetRow::Move(name) => name.0.clone(),
            SetRow::Gesture(name) => format!("{name} · gesture"),
        }
    }

    /// Whether the clip that plays here lands a hit, and so must mark its apex.
    pub fn strikes(&self) -> bool {
        matches!(self, SetRow::Action(Action::Attack) | SetRow::Move(_))
    }

    /// Whether the clip that plays here covers ground, and so states its pace.
    fn gait(&self) -> bool {
        matches!(self, SetRow::Action(action) if Action::GAITS.contains(action))
    }

    fn asked(&self) -> (Motion, Pose) {
        match self {
            SetRow::Action(action) => (Motion::Action(*action), Pose::Idle),
            SetRow::Pose(pose) => (Motion::Action(Action::Idle), *pose),
            SetRow::Begin(pose) => (Motion::Begin(*pose), Pose::Idle),
            SetRow::End(pose) => (Motion::End(*pose), Pose::Idle),
            SetRow::Move(name) => (Motion::Move(name.clone()), Pose::Idle),
            SetRow::Gesture(name) => (Motion::Gesture(name.clone()), Pose::Idle),
        }
    }

    /// The cell `set` names for the row.
    pub fn get<'s>(&self, set: &'s ClipSet) -> Option<&'s ClipCell> {
        match self {
            SetRow::Action(action) => set.clips.get(action),
            SetRow::Pose(pose) => set.poses.get(pose),
            SetRow::Begin(pose) => set.begins.get(pose),
            SetRow::End(pose) => set.ends.get(pose),
            SetRow::Move(name) => set.moves.get(name),
            SetRow::Gesture(name) => set.gestures.get(name),
        }
    }

    /// Names `cell` for the row in `set`, or takes the row's entry away with none.
    fn put(&self, set: &mut ClipSet, cell: Option<ClipCell>) -> Result<(), String> {
        fn put<K: Ord + Clone>(
            map: &mut std::collections::BTreeMap<K, ClipCell>,
            key: &K,
            cell: Option<ClipCell>,
        ) {
            match cell {
                Some(cell) => map.insert(key.clone(), cell),
                None => map.remove(key),
            };
        }
        match self {
            SetRow::Pose(Pose::Idle) | SetRow::Begin(Pose::Idle) | SetRow::End(Pose::Idle) => {
                return Err("idling plays the idle action's clip, not a pose's".to_owned());
            }
            SetRow::Action(action) => put(&mut set.clips, action, cell),
            SetRow::Pose(pose) => put(&mut set.poses, pose, cell),
            SetRow::Begin(pose) => put(&mut set.begins, pose, cell),
            SetRow::End(pose) => put(&mut set.ends, pose, cell),
            SetRow::Move(name) => put(&mut set.moves, name, cell),
            SetRow::Gesture(name) => put(&mut set.gestures, name, cell),
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct SetsState {
    /// The build whose cells the table shows; none for the first.
    pub build: Option<BuildName>,
    /// The cell open below the table: its row, and its column's stance by path (none for base).
    pub open: Option<(SetRow, Option<String>)>,
    /// Move and gesture rows added that no set names yet.
    pub added: Vec<SetRow>,
    pub new_move: String,
    pub new_gesture: String,
    pub filter: String,
    /// Whether a clip picked joins the open cell's clips rather than taking its place.
    pub adding: bool,
    /// The replacements Fill from file names rewrites the first build's paths by, while it is open.
    pub fill: Option<Vec<(String, String)>>,
}

/// Names `cell` for `row` in the set of `build` (none for the first) and `stance` (by path; none for
/// base), or takes the entry away with none so the ladder finds it further down.
pub fn set_cell(
    editor: &mut Editor,
    key: &str,
    build: Option<&BuildName>,
    stance: Option<&str>,
    row: &SetRow,
    cell: Option<ClipCell>,
) -> bool {
    let stance = match stance {
        Some(path) => match stance_id(editor, path) {
            Some(id) => Some(id),
            None => {
                editor.toast(format!("{path} is not a stance"));
                return false;
            }
        },
        None => None,
    };
    let label = format!("Set {} for {}", row.label(), cell_label(cell.as_ref()));
    super::change(editor, key, &label, |skeleton| {
        if stance.is_some_and(|stance| skeleton.set(stance).is_none()) && !is_first(skeleton, build)
        {
            return Err("a build's stance columns are the skeleton's".to_owned());
        }
        row.put(set_mut(skeleton, build, stance), cell)
    })
}

/// Gives the skeleton an empty set for the stance at `stance`, a column of the table every build has.
pub fn add_stance_column(editor: &mut Editor, key: &str, stance: &str) -> bool {
    let Some(id) = stance_id(editor, stance) else {
        editor.toast(format!("{stance} is not a stance"));
        return false;
    };
    super::change(editor, key, "Add a stance column", |skeleton| {
        if skeleton.stances.iter().any(|set| set.stance == id) {
            return Err("the skeleton has a set for that stance already".to_owned());
        }
        skeleton.stances.push(StanceSet {
            stance: id,
            set: Default::default(),
        });
        Ok(())
    })
}

/// Fills every cell `build` leaves empty from the first build's, each of whose clips' files
/// `replace` rewrites into a clip that plays, which takes the cues and pace of the clip it was
/// rewritten from where it has none of its own; gives how many cells it filled.
pub fn fill(
    editor: &mut Editor,
    key: &str,
    rigging: &Rigging,
    build: &BuildName,
    replace: &[(String, String)],
) -> usize {
    let Some(Ok(def)) = editor.read::<SkeletonDef>(key) else {
        return 0;
    };
    let fills = fills(&def, rigging, build, replace);
    let count = fills.len();
    if count == 0 {
        return 0;
    }
    super::change(
        editor,
        key,
        &format!("Fill {build} from file names"),
        |skeleton| {
            for fill in fills {
                fill.row
                    .put(set_mut(skeleton, Some(build), fill.stance), Some(fill.cell))?;
                for (from, to) in fill.rewritten {
                    let cues = skeleton.cues_of(&from).to_vec();
                    if !cues.is_empty() && skeleton.cues_of(&to).is_empty() {
                        skeleton.cues.push(ClipCues {
                            clip: to.clone(),
                            cues,
                        });
                    }
                    if let Some(pace) = skeleton
                        .pace(&from)
                        .filter(|_| skeleton.pace(&to).is_none())
                    {
                        skeleton.paces.push(ClipPace { clip: to, pace });
                    }
                }
            }
            skeleton.cues.sort_by(|a, b| a.clip.cmp(&b.clip));
            skeleton.paces.sort_by(|a, b| a.clip.cmp(&b.clip));
            Ok(())
        },
    );
    count
}

/// A cell a build leaves empty, filled from the first build's: its stance (none for base), its
/// row, and each clip it was rewritten from with the clip it became.
struct Fill {
    stance: Option<StanceId>,
    row: SetRow,
    cell: ClipCell,
    rewritten: Vec<(Clip, Clip)>,
}

/// Each cell `build` leaves empty that the first build's fills by `replace`.
fn fills(
    def: &SkeletonDef,
    rigging: &Rigging,
    build: &BuildName,
    replace: &[(String, String)],
) -> Vec<Fill> {
    let rewrite = |clip: &Clip| -> Option<Clip> {
        let mut path = clip.file().as_str().to_owned();
        for (from, to) in replace.iter().filter(|(from, _)| !from.is_empty()) {
            path = path.replace(from.as_str(), to);
        }
        let file = engine::core::assets::ModelRef::new(path);
        let rewritten = match clip.take() {
            Some(take) => Clip::Take {
                file,
                take: take.clone(),
            },
            None => Clip::File(file),
        };
        (rewritten != *clip && rigging.playable(&rewritten).is_some()).then_some(rewritten)
    };
    let columns = std::iter::once((None, &def.base))
        .chain(def.stances.iter().map(|set| (Some(set.stance), &set.set)));
    let mut filled = Vec::new();
    for (stance, set) in columns {
        let own = own_set(def, Some(build), stance);
        for row in every_row(set) {
            if own.is_some_and(|own| row.get(own).is_some()) {
                continue;
            }
            let Some(cell) = row.get(set) else {
                continue;
            };
            let rewritten: Option<Vec<(Clip, Clip)>> = cell
                .clips()
                .iter()
                .map(|named| Some((named.clip().clone(), rewrite(named.clip())?)))
                .collect();
            if let Some(rewritten) = rewritten {
                let clips = cell
                    .clips()
                    .iter()
                    .zip(&rewritten)
                    .map(|(named, (_, to))| clip_ref(to.clone(), named.only().cloned()))
                    .collect();
                filled.push(Fill {
                    stance,
                    row,
                    cell: cell_of(clips, cell.order()),
                    rewritten,
                });
            }
        }
    }
    filled
}

/// Every row `set` names a cell for.
fn every_row(set: &ClipSet) -> Vec<SetRow> {
    set.clips
        .keys()
        .map(|action| SetRow::Action(*action))
        .chain(set.poses.keys().map(|pose| SetRow::Pose(*pose)))
        .chain(set.begins.keys().map(|pose| SetRow::Begin(*pose)))
        .chain(set.ends.keys().map(|pose| SetRow::End(*pose)))
        .chain(set.moves.keys().cloned().map(SetRow::Move))
        .chain(set.gestures.keys().cloned().map(SetRow::Gesture))
        .collect()
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rigging: &Rigging) {
    let Some(content) = editor.loaded() else {
        return;
    };
    let build = build_shown(editor, key, def);
    let columns: Vec<Option<StanceId>> = std::iter::once(None)
        .chain(def.stances.iter().map(|set| Some(set.stance)))
        .collect();
    let rows = rows(editor, key, def);
    toolbar(editor, ui, key, &content, def, build.as_ref());
    fill_window(editor, ui, key, def, rigging);
    ui.add_space(6.0);
    let open = editor.views.skeleton.view(key).sets.open.clone();
    let mut picked: Option<(SetRow, Option<String>)> = None;
    let height = ui.available_height();
    egui::ScrollArea::both()
        .id_salt(("sets", key))
        .max_height(if open.is_some() {
            height * 0.42
        } else {
            height
        })
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new(("sets-table", key))
                .num_columns(columns.len() + 1)
                .spacing(Vec2::new(10.0, 4.0))
                .striped(true)
                .show(ui, |ui| {
                    ui.label("");
                    for column in &columns {
                        ui.label(RichText::new(column_name(&content, *column)).strong());
                    }
                    ui.end_row();
                    for row in &rows {
                        ui.label(theme::mono(row.label()));
                        for column in &columns {
                            let path = column.map(|stance| stance.path(&content).to_owned());
                            let lit = open
                                .as_ref()
                                .is_some_and(|(at, on)| at == row && *on == path);
                            if cell(ui, def, rigging, build.as_ref(), *column, row, lit).clicked() {
                                picked = Some((row.clone(), path));
                            }
                        }
                        ui.end_row();
                    }
                });
        });
    if let Some(picked) = picked {
        let view = editor.views.skeleton.view_mut(key);
        let sets = &mut view.sets;
        sets.open = (sets.open.as_ref() != Some(&picked)).then_some(picked);
        sets.filter.clear();
        view.clip = None;
    }
    if let Some((row, stance)) = editor.views.skeleton.view(key).sets.open.clone() {
        ui.add_space(6.0);
        form::rule(ui);
        let width = ui.available_width();
        let [left, right] = panel::split(ui, (width * 0.5).clamp(320.0, 640.0));
        panel::body(ui, left, |ui| {
            picker(
                editor,
                ui,
                key,
                &content,
                def,
                rigging,
                build.as_ref(),
                &row,
                stance.as_deref(),
            );
        });
        panel::body(ui, right, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("set-playing", key))
                .auto_shrink([false, false])
                .show(ui, |ui| playing(editor, ui, key, def, rigging, &row));
        });
    }
}

/// The build the table shows, as picked, or the first.
fn build_shown(editor: &mut Editor, key: &str, def: &SkeletonDef) -> Option<BuildName> {
    let sets = &mut editor.views.skeleton.view_mut(key).sets;
    if sets
        .build
        .as_ref()
        .is_some_and(|build| !def.builds.contains(build))
    {
        sets.build = None;
    }
    sets.build.clone().or_else(|| def.builds.first().cloned())
}

fn toolbar(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    content: &Content,
    def: &SkeletonDef,
    build: Option<&BuildName>,
) {
    let missing: Vec<(String, String)> = content
        .table::<StanceDef>()
        .iter()
        .filter(|(id, _)| def.set(*id).is_none())
        .map(|(id, stance)| (id.path(content).to_owned(), stance.shown_name().to_owned()))
        .collect();
    let mut column: Option<String> = None;
    let mut added: Option<SetRow> = None;
    let mut picked_build: Option<BuildName> = None;
    let mut filling = false;
    if !def.builds.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::muted("build"));
            for each in &def.builds {
                if chip::mode(ui, each.as_str(), Some(each) == build).clicked() {
                    picked_build = Some(each.clone());
                }
            }
            if build.is_some_and(|build| !is_first(def, Some(build)))
                && ui
                    .button("Fill from file names…")
                    .on_hover_text(format!(
                        "fills this build's empty cells from {}'s, its clips' paths rewritten",
                        def.builds[0]
                    ))
                    .clicked()
            {
                filling = true;
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(!missing.is_empty(), |ui| {
            ui.menu_button("+ Add stance column", |ui| {
                for (path, name) in &missing {
                    if ui.button(name).on_hover_text(path).clicked() {
                        column = Some(path.clone());
                        ui.close();
                    }
                }
            });
        });
        ui.add_space(12.0);
        let sets = &mut editor.views.skeleton.view_mut(key).sets;
        ui.add(
            egui::TextEdit::singleline(&mut sets.new_move)
                .hint_text("move name")
                .desired_width(140.0),
        );
        let name = sets.new_move.trim().to_owned();
        if ui
            .add_enabled(!name.is_empty(), egui::Button::new("+ Add move row"))
            .clicked()
        {
            added = Some(SetRow::Move(MoveName::new(name)));
            sets.new_move.clear();
        }
        ui.add_space(12.0);
        ui.add(
            egui::TextEdit::singleline(&mut sets.new_gesture)
                .hint_text("gesture name")
                .desired_width(140.0),
        );
        let name = sets.new_gesture.trim().to_owned();
        if ui
            .add_enabled(!name.is_empty(), egui::Button::new("+ Add gesture row"))
            .clicked()
        {
            added = Some(SetRow::Gesture(GestureName::new(name)));
            sets.new_gesture.clear();
        }
    });
    let sets = &mut editor.views.skeleton.view_mut(key).sets;
    if let Some(build) = picked_build {
        sets.build = Some(build);
        sets.open = None;
        sets.fill = None;
    }
    if filling {
        sets.fill = Some(vec![(String::new(), String::new()); 2]);
    }
    if let Some(row) = added
        && !sets.added.contains(&row)
    {
        sets.added.push(row);
    }
    if let Some(stance) = column {
        add_stance_column(editor, key, &stance);
    }
}

/// The window asking what to replace in the first build's clips' paths, with what it would fill.
fn fill_window(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rigging: &Rigging) {
    let Some(build) = editor.views.skeleton.view(key).sets.build.clone() else {
        return;
    };
    let Some(mut replace) = editor.views.skeleton.view(key).sets.fill.clone() else {
        return;
    };
    let mut open = true;
    let mut filled = false;
    egui::Window::new(format!("Fill {build} from file names"))
        .id(egui::Id::new(("fill", key)))
        .collapsible(false)
        .resizable(false)
        .min_width(460.0)
        .open(&mut open)
        .show(ui.ctx(), |ui| {
            ui.label(theme::muted(format!(
                "Each path in {}'s cells is rewritten by every replacement, in order. A cell fills \
                 where {build} has none and every rewritten path is a clip that plays.",
                def.builds[0]
            )));
            let mut removed = None;
            for (index, (from, to)) in replace.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let size = Vec2::new(180.0, ui.spacing().interact_size.y);
                    ui.add_sized(size, egui::TextEdit::singleline(from).hint_text("find"));
                    ui.add_sized(
                        size,
                        egui::TextEdit::singleline(to).hint_text("replace with"),
                    );
                    if controls::remove(ui, "replacement").clicked() {
                        removed = Some(index);
                    }
                });
            }
            if let Some(index) = removed {
                replace.remove(index);
            }
            if ui.button("+ another").clicked() {
                replace.push((String::new(), String::new()));
            }
            let found = fills(def, rigging, &build, &replace);
            ui.label(
                RichText::new(format!("fills {}", words::counted(found.len(), "cell")))
                    .color(theme::OK),
            );
            if ui
                .add_enabled(!found.is_empty(), egui::Button::new("Fill"))
                .clicked()
            {
                filled = true;
            }
        });
    if filled {
        let count = fill(editor, key, rigging, &build, &replace);
        editor.toast(format!("Filled {}", words::counted(count, "cell")));
        editor.views.skeleton.view_mut(key).sets.fill = None;
    } else {
        editor.views.skeleton.view_mut(key).sets.fill = open.then_some(replace);
    }
}

/// The table's rows: the engine actions, the poses other than idling, every move a set names or
/// the designer added, and the gestures of the generic expressions, every other a set names and
/// those the designer added.
fn rows(editor: &Editor, key: &str, def: &SkeletonDef) -> Vec<SetRow> {
    let added = &editor.views.skeleton.view(key).sets.added;
    let mut moves: Vec<MoveName> = def.moves().cloned().collect();
    let mut gestures: Vec<GestureName> = def.gestures().cloned().collect();
    for row in added {
        match row {
            SetRow::Move(name) => moves.push(name.clone()),
            SetRow::Gesture(name) => gestures.push(name.clone()),
            _ => {}
        }
    }
    moves.sort();
    moves.dedup();
    let generic: Vec<GestureName> = GenericExpression::VARIANTS
        .iter()
        .map(|expression| GestureName::new(expression.as_ref()))
        .collect();
    gestures.retain(|name| !generic.contains(name));
    gestures.sort();
    gestures.dedup();
    Action::VARIANTS
        .iter()
        .map(|action| SetRow::Action(*action))
        .chain(
            Pose::VARIANTS
                .iter()
                .filter(|pose| **pose != Pose::Idle)
                .flat_map(|pose| {
                    [
                        SetRow::Begin(*pose),
                        SetRow::Pose(*pose),
                        SetRow::End(*pose),
                    ]
                }),
        )
        .chain(moves.into_iter().map(SetRow::Move))
        .chain(generic.into_iter().chain(gestures).map(SetRow::Gesture))
        .collect()
}

fn cell(
    ui: &mut Ui,
    def: &SkeletonDef,
    rigging: &Rigging,
    build: Option<&BuildName>,
    column: Option<StanceId>,
    row: &SetRow,
    lit: bool,
) -> egui::Response {
    let (motion, pose) = row.asked();
    let found = def.named(build, column, &motion, pose);
    let own = own_set(def, build, column).is_some_and(|set| row.get(set).is_some());
    let missing = found.is_none() && matches!(row, SetRow::Action(action) if action.required());
    let mut text = RichText::new(cell_label(found.map(|(cell, _)| cell))).size(12.5);
    if !own {
        text = text.italics().color(theme::FAINT);
    }
    let unknown = found.is_some_and(|(cell, _)| {
        cell.clips()
            .iter()
            .any(|clip| rigging.playable(clip.clip()).is_none())
    });
    if unknown || missing {
        text = text.color(theme::DANGER);
    }
    let response = ui.add(egui::Button::selectable(lit, text));
    match (found, unknown, missing) {
        (_, _, true) => response.on_hover_text(
            "no body wears the skeleton until base has a clip for every engine action",
        ),
        (_, true, _) => response.on_hover_text("a clip here doesn't play on this skeleton"),
        (Some((_, Rung::Fallback)), _, _) => response.on_hover_text(match row {
            SetRow::Gesture(_) => "no set names this gesture: the body makes none",
            _ => "no set names this move: the attack plays",
        }),
        _ => response,
    }
}

/// The open cell: its clips and how they take turns, and every clip that plays to pick from.
#[allow(clippy::too_many_arguments)]
fn picker(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    content: &Content,
    def: &SkeletonDef,
    rigging: &Rigging,
    build: Option<&BuildName>,
    row: &SetRow,
    stance: Option<&str>,
) {
    let column = stance.and_then(|path| content.by_path::<StanceDef>(path));
    let current = own_set(def, build, column).and_then(|set| row.get(set));
    let mut chosen: Option<Option<ClipCell>> = None;
    ui.horizontal(|ui| {
        let mut title = format!("{} · {}", row.label(), column_name(content, column));
        if let Some(build) = build.filter(|_| def.builds.len() > 1) {
            title.push_str(&format!(" · {build}"));
        }
        ui.label(RichText::new(title).strong());
        if ui.button("close").clicked() {
            editor.views.skeleton.view_mut(key).sets.open = None;
        }
    });
    if current.is_some() {
        let below = {
            let mut without = def.clone();
            row.put(set_mut(&mut without, build, column), None).ok();
            let (motion, pose) = row.asked();
            cell_label(
                without
                    .named(build, column, &motion, pose)
                    .map(|(cell, _)| cell),
            )
        };
        if ui
            .button(RichText::new(format!("— fall back: plays {below}")).italics())
            .clicked()
        {
            chosen = Some(None);
        }
    }
    let clips: Vec<ClipRef> = current.map_or(Vec::new(), |cell| cell.clips().to_vec());
    let order = current.map_or(ClipOrder::Random, ClipCell::order);
    let playing = editor.views.skeleton.view(key).clip.clone();
    let mut play: Option<Clip> = None;
    if !clips.is_empty() {
        let mut kept = clips.clone();
        let mut removed = false;
        for (index, clip) in clips.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(theme::faint(format!("{}.", index + 1)));
                let lit = playing.as_ref() == Some(clip.clip());
                if ui
                    .add(egui::Button::selectable(
                        lit,
                        theme::mono(clip_label(Some(clip))).size(12.5),
                    ))
                    .on_hover_text("play it on the stage")
                    .clicked()
                {
                    play = Some(clip.clip().clone());
                }
                if controls::remove(ui, "clip").clicked() {
                    kept.remove(index);
                    removed = true;
                }
            });
        }
        let mut picked = order;
        if clips.len() > 1 {
            ui.horizontal(|ui| {
                ui.label(theme::muted("taking turns"));
                controls::combo(("set-order", key))
                    .selected_text(order_label(order))
                    .show_ui(ui, |ui| {
                        for order in [ClipOrder::Random, ClipOrder::InTurn] {
                            ui.selectable_value(&mut picked, order, order_label(order));
                        }
                    });
            });
        }
        if removed && kept.is_empty() {
            chosen = Some(None);
        } else if removed || picked != order {
            chosen = Some(Some(cell_of(kept, picked)));
        }
    }
    let groups: Vec<GroupName> = def.groups.keys().cloned().collect();
    let mut only = clips.first().and_then(ClipRef::only).cloned();
    if !groups.is_empty() {
        ui.horizontal(|ui| {
            ui.label(theme::muted("kept to"));
            controls::combo(("set-only", key))
                .selected_text(
                    only.as_ref()
                        .map_or("the whole body".to_owned(), |g| g.0.clone()),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut only, None, "the whole body");
                    for group in &groups {
                        ui.selectable_value(&mut only, Some(group.clone()), &group.0);
                    }
                });
        });
        if !clips.is_empty() && clips.iter().any(|clip| clip.only() != only.as_ref()) {
            let regrouped = clips
                .iter()
                .map(|clip| clip_ref(clip.clip().clone(), only.clone()))
                .collect();
            chosen = Some(Some(cell_of(regrouped, order)));
        }
    }
    if current.is_some() {
        let adding = &mut editor.views.skeleton.view_mut(key).sets.adding;
        ui.checkbox(adding, "a clip picked joins the cell's clips");
    }
    let adding = editor.views.skeleton.view(key).sets.adding && current.is_some();
    let filter = &mut editor.views.skeleton.view_mut(key).sets.filter;
    ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text("search clips by any part of the path")
            .desired_width(f32::INFINITY),
    );
    let filter = filter.clone();
    let found: Vec<_> = super::matching(rigging, &filter).collect();
    ui.label(theme::faint(format!(
        "{} play on this skeleton",
        words::counted(found.len(), "clip")
    )));
    egui::ScrollArea::vertical()
        .id_salt(("set-picker", key))
        .auto_shrink([false, false])
        .show_rows(ui, 20.0, found.len(), |ui, shown| {
            for found in &found[shown] {
                let apex = def.cues_of(&found.clip).iter().any(|cue| cue.apex);
                let lit = clips.iter().any(|clip| *clip.clip() == found.clip);
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::selectable(
                            lit,
                            theme::mono(found.clip.to_string()).size(12.0),
                        ))
                        .clicked()
                    {
                        let picked = clip_ref(found.clip.clone(), only.clone());
                        chosen = Some(Some(if adding {
                            let mut grown = clips.clone();
                            grown.push(picked);
                            cell_of(grown, order)
                        } else {
                            ClipCell::One(picked)
                        }));
                        play = Some(found.clip.clone());
                    }
                    let mut note = format!("{:.2} s", found.length.0);
                    if row.strikes() {
                        note.push_str(if apex { " · apex" } else { " · no apex" });
                    }
                    ui.label(theme::faint(note).size(12.0));
                });
            }
        });
    if let Some(cell) = chosen {
        set_cell(editor, key, build, stance, row, cell);
    }
    if let Some(clip) = play {
        let view = editor.views.skeleton.view_mut(key);
        view.clip = Some(clip);
        view.scrub.restart();
    }
}

/// The clip chosen in the open cell on the stage, its timeline to mark its cues on, where else it
/// plays and, for a gait, its pace.
fn playing(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    def: &SkeletonDef,
    rigging: &Rigging,
    row: &SetRow,
) {
    let Some(clip) = editor.views.skeleton.view(key).clip.clone() else {
        form::hint(
            ui,
            "Click a clip of the cell to play it here and mark its cues.",
        );
        return;
    };
    let Some(found) = rigging.playable(&clip).cloned() else {
        ui.colored_label(
            theme::DANGER,
            format!("{clip} doesn't play on this skeleton"),
        );
        return;
    };
    let props = match editor.loaded() {
        Some(content) => {
            super::item_picker(editor, ui, key, &content, "holding");
            super::holding(editor, key, &content)
        }
        None => Vec::new(),
    };
    let paced = row.gait() || def.pace(&clip).is_some();
    let below = if paced { 270.0 } else { 150.0 };
    let size = Vec2::new(
        ui.available_width(),
        (ui.available_height() - below).max(140.0),
    );
    super::stage_rig(editor, ui, key, def, rigging, size, props);
    ui.label(RichText::new(format!("{clip} · {:.2} s", found.length.0)).strong());
    let held = def.cues_of(&clip).to_vec();
    let scrub = &mut editor.views.skeleton.view_mut(key).scrub;
    if let Some(cues) = cues::timeline(ui, scrub, found.length.millis(), &held, true) {
        super::set_cues(editor, key, &clip, cues);
    }
    if row.strikes() && !held.iter().any(|cue| cue.apex) {
        ui.colored_label(
            theme::DANGER,
            "a hit lands at its apex: pause where it lands and press A",
        );
    }
    let also = uses(def, &clip);
    if also.len() > 1 {
        ui.label(theme::faint(format!(
            "its cues serve every cell that plays it: {}",
            also.join(" · ")
        )));
    }
    if paced {
        pace(editor, ui, key, def, &clip);
    }
    editor.status.left.push(cues::KEYS.to_owned());
}

/// The ground the clip covers at scale 1.
fn pace(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, clip: &Clip) {
    let held = def.pace(clip);
    let mut value = held.map_or(0.0, |pace| pace.0.0);
    let dragged = ui
        .horizontal(|ui| {
            ui.label(theme::muted("pace · m/s"));
            let dragged = ui
                .add(
                    egui::DragValue::new(&mut value)
                        .speed(0.01)
                        .range(0.01..=20.0)
                        .fixed_decimals(2),
                )
                .changed();
            if held.is_none() {
                ui.label(theme::faint("none yet: a gait must state one"));
            }
            dragged
        })
        .inner;
    form::hint(
        ui,
        "How fast the clip carries a body at scale 1: the speed its planted feet slide back under \
         it. A body plays the gait nearest its speed, sped up or slowed to cover it exactly.",
    );
    if dragged {
        super::set_pace(editor, key, clip, MetersPerSec(Meters(value)));
    }
}

/// Every cell that plays `clip`, as its set and entry.
pub fn uses(def: &SkeletonDef, clip: &Clip) -> Vec<String> {
    def.sets()
        .flat_map(|(set, cells)| {
            cells
                .entries()
                .filter(|(_, named)| named.clip() == clip)
                .map(move |(entry, _)| format!("{set}{entry}"))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether `build` is the first of the skeleton's, whose cells are the sets every build falls back
/// to: none stands for it.
fn is_first(def: &SkeletonDef, build: Option<&BuildName>) -> bool {
    build.is_none_or(|build| def.builds.first() == Some(build))
}

/// The set `build` names for `stance` (none for base) itself, when it has one.
fn own_set<'d>(
    def: &'d SkeletonDef,
    build: Option<&BuildName>,
    stance: Option<StanceId>,
) -> Option<&'d ClipSet> {
    if is_first(def, build) {
        return match stance {
            None => Some(&def.base),
            Some(stance) => def.set(stance),
        };
    }
    let own = def.build_sets.get(build?)?;
    match stance {
        None => Some(&own.base),
        Some(stance) => own
            .stances
            .iter()
            .find(|set| set.stance == stance)
            .map(|set| &set.set),
    }
}

/// The set `build` names for `stance` (none for base), made where it has none.
fn set_mut<'d>(
    def: &'d mut SkeletonDef,
    build: Option<&BuildName>,
    stance: Option<StanceId>,
) -> &'d mut ClipSet {
    let first = is_first(def, build);
    let (base, stances) = match (first, build) {
        (false, Some(build)) => {
            let own: &mut BuildSets = def.build_sets.entry(build.clone()).or_default();
            (&mut own.base, &mut own.stances)
        }
        _ => (&mut def.base, &mut def.stances),
    };
    let Some(stance) = stance else {
        return base;
    };
    let index = match stances.iter().position(|set| set.stance == stance) {
        Some(index) => index,
        None => {
            stances.push(StanceSet {
                stance,
                set: ClipSet::default(),
            });
            stances.len() - 1
        }
    };
    &mut stances[index].set
}

fn clip_ref(clip: Clip, only: Option<GroupName>) -> ClipRef {
    match only {
        Some(only) => ClipRef::Grouped { clip, only },
        None => ClipRef::Whole(clip),
    }
}

/// A cell of `clips` taking turns in `order`, or the one clip when there is only one.
fn cell_of(mut clips: Vec<ClipRef>, order: ClipOrder) -> ClipCell {
    match clips.len() {
        1 => ClipCell::One(clips.remove(0)),
        _ => ClipCell::Several { clips, order },
    }
}

fn cell_label(cell: Option<&ClipCell>) -> String {
    let Some(cell) = cell else {
        return "nothing".to_owned();
    };
    let names: Vec<String> = cell
        .clips()
        .iter()
        .map(|clip| clip_label(Some(clip)))
        .collect();
    match (names.len(), cell.order()) {
        (0, _) => "no clips".to_owned(),
        (1, _) => names.concat(),
        (_, order) => format!("{} · {}", names.join(" / "), order_label(order)),
    }
}

fn order_label(order: ClipOrder) -> &'static str {
    match order {
        ClipOrder::Random => "at random",
        ClipOrder::InTurn => "in turn",
    }
}

fn clip_label(clip: Option<&ClipRef>) -> String {
    match clip {
        Some(ClipRef::Whole(clip)) => clip.shown(),
        Some(ClipRef::Grouped { clip, only }) => format!("{} · {only} only", clip.shown()),
        None => "nothing".to_owned(),
    }
}

fn column_name(content: &Content, column: Option<StanceId>) -> String {
    column.map_or("base".to_owned(), |stance| {
        stance.get(content).shown_name().to_owned()
    })
}

fn stance_id(editor: &Editor, path: &str) -> Option<StanceId> {
    editor.reader()?.id_of::<StanceDef>(path)
}
