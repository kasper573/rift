use std::time::SystemTime;

use super::AreaCameraState;
use super::files::FileChange;

pub const MAX_STEPS: usize = 500;
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct EditStep {
    pub label: String,
    pub before: Vec<FileChange>,
    pub after: Vec<FileChange>,
    pub selection: RowSelection,
    pub camera: Option<AreaCameraState>,
    pub at: SystemTime,
}

impl EditStep {
    /// Takes `after` into the step, keeping from `before` what a file it didn't touch was.
    fn fold(&mut self, after: Vec<FileChange>, before: &[FileChange]) {
        for change in after {
            match self
                .after
                .iter_mut()
                .find(|known| known.file == change.file)
            {
                Some(known) => known.bytes = change.bytes,
                None => {
                    if let Some(was) = before.iter().find(|was| was.file == change.file) {
                        self.before.push(was.clone());
                    }
                    self.after.push(change);
                }
            }
        }
        self.at = SystemTime::now();
    }
}

pub const GAME: &str = "game";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowSelection {
    pub table: String,
    pub keys: Vec<String>,
    pub parts: Vec<String>,
}

impl RowSelection {
    pub fn rows(table: &str, keys: Vec<String>) -> RowSelection {
        RowSelection {
            table: table.to_owned(),
            keys,
            parts: Vec::new(),
        }
    }

    pub fn row(table: &str, key: &str) -> RowSelection {
        RowSelection::rows(table, vec![key.to_owned()])
    }

    pub fn parts(table: &str, key: &str, parts: Vec<String>) -> RowSelection {
        RowSelection {
            table: table.to_owned(),
            keys: vec![key.to_owned()],
            parts,
        }
    }

    pub fn game() -> RowSelection {
        RowSelection::row(GAME, GAME)
    }
}

#[derive(Clone)]
pub struct EditHistory {
    steps: Vec<EditStep>,
    applied: usize,
    saved: Option<usize>,
}

impl Default for EditHistory {
    fn default() -> Self {
        EditHistory {
            steps: Vec::new(),
            applied: 0,
            saved: Some(0),
        }
    }
}

impl EditHistory {
    pub fn push(&mut self, step: EditStep) {
        self.steps.truncate(self.applied);
        if self.saved.is_some_and(|at| at > self.applied) {
            self.saved = None;
        }
        self.steps.push(step);
        self.applied = self.steps.len();
        self.trim();
    }

    pub(crate) fn undo_step(&self) -> Option<&EditStep> {
        self.applied.checked_sub(1).map(|at| &self.steps[at])
    }

    pub(crate) fn redo_step(&self) -> Option<&EditStep> {
        self.steps.get(self.applied)
    }

    pub(crate) fn undone(&mut self) {
        self.applied = self.applied.saturating_sub(1);
    }

    pub(crate) fn redone(&mut self) {
        self.applied = (self.applied + 1).min(self.steps.len());
    }

    pub fn steps(&self) -> &[EditStep] {
        &self.steps
    }

    pub fn applied(&self) -> usize {
        self.applied
    }

    pub fn saved(&self) -> Option<usize> {
        self.saved
    }

    pub(crate) fn mark_saved(&mut self) {
        self.saved = Some(self.applied);
    }

    pub fn reset(&mut self) {
        *self = EditHistory::default();
    }

    /// Folds `after` into the last step when it carries the same label, was made within the last
    /// second and nothing was undone since; files the step didn't touch keep `before` to undo to.
    pub(crate) fn merge_into_last(
        &mut self,
        label: &str,
        after: Vec<FileChange>,
        before: &[FileChange],
    ) -> bool {
        if self.applied != self.steps.len() {
            return false;
        }
        let Some(last) = self.steps.last_mut() else {
            return false;
        };
        let recent = last
            .at
            .elapsed()
            .is_ok_and(|since| since.as_secs_f32() < 1.0);
        if last.label != label || !recent {
            return false;
        }
        last.fold(after, before);
        true
    }

    /// Folds `after` into the last step when it is labelled `step` and nothing was undone since,
    /// relabelling it; files the step didn't touch keep `before` to undo to.
    pub(crate) fn amend_last(
        &mut self,
        step: &str,
        label: &str,
        after: Vec<FileChange>,
        before: &[FileChange],
    ) -> bool {
        if self.applied != self.steps.len() {
            return false;
        }
        let Some(last) = self.steps.last_mut() else {
            return false;
        };
        if last.label != step {
            return false;
        }
        last.fold(after, before);
        last.label = label.to_owned();
        true
    }

    fn trim(&mut self) {
        while self.steps.len() > MAX_STEPS || (self.bytes() > MAX_BYTES && self.steps.len() > 1) {
            self.steps.remove(0);
            self.applied = self.applied.saturating_sub(1);
            self.saved = self.saved.and_then(|at| at.checked_sub(1));
        }
    }

    fn bytes(&self) -> usize {
        self.steps
            .iter()
            .flat_map(|step| step.before.iter().chain(step.after.iter()))
            .map(|change| change.bytes.as_ref().map_or(0, |bytes| bytes.len()))
            .sum()
    }
}
