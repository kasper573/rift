//! The sets page: what anyone can ask of the skeleton (its engine actions, poses and every move a
//! set names) against base and each stance it has a set for, each cell the clips the ladder finds.
//! A cell a set names is plain, one found further down the ladder italic; picking a cell lists every
//! library clip with its length and apex, to play alone or as one more variant of the cell.

use engine::core::content::{Content, NamedContent};
use engine::systems::actor::skeleton::{
    BuiltSkeleton, ClipCell, ClipName, ClipRef, GroupName, MoveName, Rung, SkeletonDef, StanceSet,
    VariantOrder,
};
use engine::systems::actor::stance::{StanceDef, StanceId};
use engine::systems::actor::{Action, Motion, Pose};
use strum::VariantArray;
use ui::egui::{self, RichText, Ui, Vec2};
use ui::{controls, form, theme};

use crate::app::Editor;

/// A row of the table: what is asked of the skeleton.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetRow {
    Action(Action),
    Pose(Pose),
    Begin(Pose),
    End(Pose),
    Move(MoveName),
}

impl SetRow {
    pub fn label(&self) -> String {
        match self {
            SetRow::Action(action) => action.as_ref().to_owned(),
            SetRow::Pose(pose) => format!("{} · pose", pose.as_ref()),
            SetRow::Begin(pose) => format!("{} · taking it", pose.as_ref()),
            SetRow::End(pose) => format!("{} · leaving it", pose.as_ref()),
            SetRow::Move(name) => name.0.clone(),
        }
    }

    /// Whether the clip that plays here lands a hit, and so must mark its apex.
    pub fn strikes(&self) -> bool {
        matches!(self, SetRow::Action(Action::Attack) | SetRow::Move(_))
    }

    fn asked(&self) -> (Motion, Pose) {
        match self {
            SetRow::Action(action) => (Motion::Action(*action), Pose::Idle),
            SetRow::Pose(pose) => (Motion::Action(Action::Idle), *pose),
            SetRow::Begin(pose) => (Motion::Begin(*pose), Pose::Idle),
            SetRow::End(pose) => (Motion::End(*pose), Pose::Idle),
            SetRow::Move(name) => (Motion::Move(name.clone()), Pose::Idle),
        }
    }
}

#[derive(Default)]
pub struct SetsState {
    /// The cell whose picker is open: its row, and its column's stance by path (none for base).
    pub open: Option<(SetRow, Option<String>)>,
    /// Move rows added that no set names yet.
    pub added: Vec<MoveName>,
    pub new_move: String,
    pub filter: String,
    /// Whether a clip picked joins the open cell's variants rather than taking its place.
    pub adding: bool,
}

/// Names `clip` for `row` in the set of `stance` (by path; none for base), or takes the entry away
/// with none so the ladder finds it further down. The base set keeps every engine action.
pub fn set_cell(
    editor: &mut Editor,
    key: &str,
    stance: Option<&str>,
    row: &SetRow,
    clip: Option<ClipCell>,
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
    let label = format!("Set {} for {}", row.label(), cell_label(clip.as_ref()));
    super::change(editor, key, &label, |skeleton| {
        let set = match stance {
            None => &mut skeleton.base,
            Some(stance) => {
                let index = match skeleton.stances.iter().position(|set| set.stance == stance) {
                    Some(index) => index,
                    None => {
                        skeleton.stances.push(StanceSet {
                            stance,
                            set: Default::default(),
                        });
                        skeleton.stances.len() - 1
                    }
                };
                &mut skeleton.stances[index].set
            }
        };
        match (row, clip) {
            (SetRow::Action(action), None) if stance.is_none() => Err(format!(
                "the base set keeps a clip for {}: every engine action needs one",
                action.as_ref()
            )),
            (SetRow::Action(action), Some(clip)) => {
                set.clips.insert(*action, clip);
                Ok(())
            }
            (SetRow::Action(action), None) => {
                set.clips.remove(action);
                Ok(())
            }
            (SetRow::Pose(Pose::Idle) | SetRow::Begin(Pose::Idle) | SetRow::End(Pose::Idle), _) => {
                Err("idling plays the idle action's clip, not a pose's".to_owned())
            }
            (SetRow::Begin(pose), Some(clip)) => {
                set.begins.insert(*pose, clip);
                Ok(())
            }
            (SetRow::Begin(pose), None) => {
                set.begins.remove(pose);
                Ok(())
            }
            (SetRow::End(pose), Some(clip)) => {
                set.ends.insert(*pose, clip);
                Ok(())
            }
            (SetRow::End(pose), None) => {
                set.ends.remove(pose);
                Ok(())
            }
            (SetRow::Pose(pose), Some(clip)) => {
                set.poses.insert(*pose, clip);
                Ok(())
            }
            (SetRow::Pose(pose), None) => {
                set.poses.remove(pose);
                Ok(())
            }
            (SetRow::Move(name), Some(clip)) => {
                set.moves.insert(name.clone(), clip);
                Ok(())
            }
            (SetRow::Move(name), None) => {
                set.moves.remove(name);
                Ok(())
            }
        }
    })
}

/// Gives the skeleton an empty set for the stance at `stance`, a column of the table.
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

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, content: &Content, built: &BuiltSkeleton) {
    let def = &built.def;
    let columns: Vec<Option<StanceId>> = std::iter::once(None)
        .chain(def.stances.iter().map(|set| Some(set.stance)))
        .collect();
    let rows = rows(editor, key, def);
    form::hint(
        ui,
        "Rows are what anyone can ask of this skeleton: the engine actions, its poses, and every \
         move a set here names. Columns are base and each stance it has a set for. Italic cells are \
         found further down the ladder.",
    );
    toolbar(editor, ui, key, content, def);
    ui.add_space(6.0);
    let open = editor.views.skeleton.view(key).sets.open.clone();
    let mut picked: Option<(SetRow, Option<String>)> = None;
    let height = ui.available_height();
    egui::ScrollArea::both()
        .id_salt(("sets", key))
        .max_height(if open.is_some() {
            height * 0.55
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
                        ui.label(RichText::new(column_name(content, *column)).strong());
                    }
                    ui.end_row();
                    for row in &rows {
                        ui.label(theme::mono(row.label()));
                        for column in &columns {
                            let path = column.map(|stance| stance.path(content).to_owned());
                            let lit = open
                                .as_ref()
                                .is_some_and(|(at, on)| at == row && *on == path);
                            if cell(ui, def, built, *column, row, lit).clicked() {
                                picked = Some((row.clone(), path));
                            }
                        }
                        ui.end_row();
                    }
                });
        });
    if let Some(picked) = picked {
        let sets = &mut editor.views.skeleton.view_mut(key).sets;
        sets.open = (sets.open.as_ref() != Some(&picked)).then_some(picked);
        sets.filter.clear();
    }
    if let Some((row, stance)) = editor.views.skeleton.view(key).sets.open.clone() {
        ui.add_space(6.0);
        form::rule(ui);
        picker(editor, ui, key, content, built, &row, stance.as_deref());
    }
}

fn toolbar(editor: &mut Editor, ui: &mut Ui, key: &str, content: &Content, def: &SkeletonDef) {
    let missing: Vec<(String, String)> = content
        .table::<StanceDef>()
        .iter()
        .filter(|(id, _)| def.set(*id).is_none())
        .map(|(id, stance)| (id.path(content).to_owned(), stance.shown_name().to_owned()))
        .collect();
    let mut column: Option<String> = None;
    let mut added: Option<MoveName> = None;
    ui.horizontal(|ui| {
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
            added = Some(MoveName::new(name));
            sets.new_move.clear();
        }
    });
    if let Some(stance) = column {
        add_stance_column(editor, key, &stance);
    }
    if let Some(name) = added {
        let sets = &mut editor.views.skeleton.view_mut(key).sets;
        if !def.moves().any(|declared| *declared == name) && !sets.added.contains(&name) {
            sets.added.push(name);
        }
    }
}

/// The table's rows: the engine actions, the poses other than idling, and every move a set names
/// or the designer added.
fn rows(editor: &Editor, key: &str, def: &SkeletonDef) -> Vec<SetRow> {
    let mut moves: Vec<MoveName> = def.moves().cloned().collect();
    moves.extend(editor.views.skeleton.view(key).sets.added.iter().cloned());
    moves.sort();
    moves.dedup();
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
        .collect()
}

fn cell(
    ui: &mut Ui,
    def: &SkeletonDef,
    built: &BuiltSkeleton,
    column: Option<StanceId>,
    row: &SetRow,
    lit: bool,
) -> egui::Response {
    let (motion, pose) = row.asked();
    let (clip, rung) = def.named(column, &motion, pose);
    let own = match column {
        Some(_) => rung == Rung::Stance,
        None => rung == Rung::Base && named_in(&def.base, row),
    };
    let mut text = RichText::new(cell_label(Some(clip))).size(12.5);
    if !own {
        text = text.italics().color(theme::FAINT);
    }
    let unknown = clip
        .variants()
        .iter()
        .any(|variant| !built.clips.contains_key(variant.clip()));
    if unknown {
        text = text.color(theme::DANGER);
    }
    let response = ui.add(egui::Button::selectable(lit, text));
    match (rung, unknown) {
        (_, true) => response.on_hover_text("no library carries this clip; a model must"),
        (Rung::Fallback, _) => response.on_hover_text("no set names this move: the attack plays"),
        _ => response,
    }
}

fn named_in(set: &engine::systems::actor::skeleton::ClipSet, row: &SetRow) -> bool {
    match row {
        SetRow::Action(action) => set.clips.contains_key(action),
        SetRow::Pose(pose) => set.poses.contains_key(pose),
        SetRow::Begin(pose) => set.begins.contains_key(pose),
        SetRow::End(pose) => set.ends.contains_key(pose),
        SetRow::Move(name) => set.moves.contains_key(name),
    }
}

/// The open cell's picker: every library clip with its length and apex, and the way back to what
/// the ladder finds below.
fn picker(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    content: &Content,
    built: &BuiltSkeleton,
    row: &SetRow,
    stance: Option<&str>,
) {
    let def = &built.def;
    let column = stance.and_then(|path| content.by_path::<StanceDef>(path));
    let set = match column {
        Some(stance) => def.set(stance),
        None => Some(&def.base),
    };
    let current = set.and_then(|set| match row {
        SetRow::Action(action) => set.clips.get(action),
        SetRow::Pose(pose) => set.poses.get(pose),
        SetRow::Begin(pose) => set.begins.get(pose),
        SetRow::End(pose) => set.ends.get(pose),
        SetRow::Move(name) => set.moves.get(name),
    });
    let mut chosen: Option<Option<ClipCell>> = None;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{} · {}",
                row.label(),
                column_name(content, column)
            ))
            .strong(),
        );
        if ui.button("close").clicked() {
            editor.views.skeleton.view_mut(key).sets.open = None;
        }
    });
    let removable = column.is_some() || !matches!(row, SetRow::Action(_));
    if removable && current.is_some() {
        let below = {
            let mut without = def.clone();
            match column.and_then(|stance| without.stances.iter_mut().find(|s| s.stance == stance))
            {
                Some(set) => forget(&mut set.set, row),
                None => forget(&mut without.base, row),
            }
            let (motion, pose) = row.asked();
            cell_label(Some(without.named(column, &motion, pose).0))
        };
        if ui
            .button(RichText::new(format!("— fall back: plays {below}")).italics())
            .clicked()
        {
            chosen = Some(None);
        }
    }
    let variants: Vec<ClipRef> = current.map_or(Vec::new(), |cell| cell.variants().to_vec());
    let order = current.map_or(VariantOrder::Random, ClipCell::order);
    if variants.len() > 1 {
        let mut kept = variants.clone();
        let mut removed = false;
        for (index, variant) in variants.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(theme::faint(format!("{}.", index + 1)));
                ui.label(theme::mono(clip_label(Some(variant))).size(12.5));
                if controls::remove(ui, "variant").clicked() {
                    kept.remove(index);
                    removed = true;
                }
            });
        }
        let mut picked = order;
        ui.horizontal(|ui| {
            ui.label(theme::muted("taking turns"));
            controls::combo(("set-order", key))
                .selected_text(order_label(order))
                .show_ui(ui, |ui| {
                    for order in [VariantOrder::Random, VariantOrder::InTurn] {
                        ui.selectable_value(&mut picked, order, order_label(order));
                    }
                });
        });
        if removed || picked != order {
            chosen = Some(Some(cell_of(kept, picked)));
        }
    }
    let groups: Vec<GroupName> = def.groups.keys().cloned().collect();
    let mut only = variants.first().and_then(ClipRef::only).cloned();
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
        if !variants.is_empty()
            && variants
                .iter()
                .any(|variant| variant.only() != only.as_ref())
        {
            let regrouped = variants
                .iter()
                .map(|variant| clip_ref(variant.clip().clone(), only.clone()))
                .collect();
            chosen = Some(Some(cell_of(regrouped, order)));
        }
    }
    if current.is_some() {
        let adding = &mut editor.views.skeleton.view_mut(key).sets.adding;
        ui.checkbox(adding, "a clip picked joins the variants");
    }
    let adding = editor.views.skeleton.view(key).sets.adding && current.is_some();
    let filter = &mut editor.views.skeleton.view_mut(key).sets.filter;
    ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text("filter clips")
            .desired_width(200.0),
    );
    let filter = filter.trim().to_lowercase();
    egui::ScrollArea::vertical()
        .id_salt(("set-picker", key))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (name, found) in &built.clips {
                if !filter.is_empty() && !name.0.to_lowercase().contains(&filter) {
                    continue;
                }
                let apex = def
                    .cues
                    .get(name)
                    .and_then(|cues| cues.iter().find(|cue| cue.apex));
                let library = &def.libraries[found.library].key;
                let lit = variants.iter().any(|variant| variant.clip() == name);
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::selectable(lit, theme::mono(name.as_str())))
                        .clicked()
                    {
                        let picked = clip_ref(name.clone(), only.clone());
                        chosen = Some(Some(if adding {
                            let mut grown = variants.clone();
                            grown.push(picked);
                            cell_of(grown, order)
                        } else {
                            ClipCell::One(picked)
                        }));
                    }
                    let mut note = format!("clips/{library}.glb · {:.2} s", found.length.0);
                    match apex {
                        Some(apex) => note.push_str(&format!(" · apex {:.0} ms", apex.at.0)),
                        None => note.push_str(" · no apex"),
                    }
                    if row.strikes() && apex.is_none() {
                        ui.colored_label(
                            theme::DANGER,
                            format!(
                                "{note}: a hit must land at an apex; mark one on the clips page"
                            ),
                        );
                    } else {
                        ui.label(theme::faint(note));
                    }
                });
            }
        });
    if let Some(clip) = chosen {
        set_cell(editor, key, stance, row, clip);
    }
}

fn forget(set: &mut engine::systems::actor::skeleton::ClipSet, row: &SetRow) {
    match row {
        SetRow::Action(action) => {
            set.clips.remove(action);
        }
        SetRow::Pose(pose) => {
            set.poses.remove(pose);
        }
        SetRow::Begin(pose) => {
            set.begins.remove(pose);
        }
        SetRow::End(pose) => {
            set.ends.remove(pose);
        }
        SetRow::Move(name) => {
            set.moves.remove(name);
        }
    }
}

fn clip_ref(clip: ClipName, only: Option<GroupName>) -> ClipRef {
    match only {
        Some(only) => ClipRef::Grouped { clip, only },
        None => ClipRef::Whole(clip),
    }
}

/// A cell of `variants` taking turns in `order`, or the one clip when there is only one.
fn cell_of(mut variants: Vec<ClipRef>, order: VariantOrder) -> ClipCell {
    match variants.len() {
        1 => ClipCell::One(variants.remove(0)),
        _ => ClipCell::Variants { variants, order },
    }
}

fn cell_label(cell: Option<&ClipCell>) -> String {
    let Some(cell) = cell else {
        return "nothing".to_owned();
    };
    let names: Vec<String> = cell
        .variants()
        .iter()
        .map(|variant| clip_label(Some(variant)))
        .collect();
    match (names.len(), cell.order()) {
        (0, _) => "no variants".to_owned(),
        (1, _) => names.concat(),
        (_, order) => format!("{} · {}", names.join(" / "), order_label(order)),
    }
}

fn order_label(order: VariantOrder) -> &'static str {
    match order {
        VariantOrder::Random => "at random",
        VariantOrder::InTurn => "in turn",
    }
}

fn clip_label(clip: Option<&ClipRef>) -> String {
    match clip {
        Some(ClipRef::Whole(clip)) => clip.0.clone(),
        Some(ClipRef::Grouped { clip, only }) => format!("{clip} · {only} only"),
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
