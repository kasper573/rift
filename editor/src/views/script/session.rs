use super::graph::{ScriptGraph, ScriptGraphLayout};
use super::{callout::ScriptCallout, doc::ScriptRow, keys::ScriptKeys, palette, run};
use crate::project::Project;
use crate::user::UserFolder;
use std::collections::{HashMap, HashSet};
use ui::egui;
use ui::egui::emath::TSTransform;
use ui::egui::{Pos2, Rect};

#[derive(Default)]
pub struct ScriptViewState {
    pub graph: ScriptGraph,
    pub outline: bool,
    pub layout_dirty: bool,
    pub selected: Vec<String>,
    pub marked: Vec<String>,
    pub palette: Option<palette::NodePalette>,
    pub outline_focus: Option<String>,
    pub folded: HashSet<String>,
    pub to_global: Option<TSTransform>,
    pub rects: HashMap<String, Rect>,
}

impl ScriptViewState {
    pub fn opened(layout: ScriptGraphLayout) -> ScriptViewState {
        ScriptViewState {
            graph: ScriptGraph::new(&layout),
            outline: layout.outline,
            ..Default::default()
        }
    }

    pub(crate) fn toggle_outline(&mut self) {
        self.outline = !self.outline;
        self.layout_dirty = true;
    }

    pub(crate) fn place_node(&mut self, name: &str, at: Pos2) {
        self.graph.place(name, at);
        self.layout_dirty = true;
    }

    pub fn rename_node(&mut self, old: &str, new: &str) {
        self.graph.rename(old, new);
        self.layout_dirty = true;
    }
}

pub(crate) fn snarl_id(at: &ScriptRow) -> egui::Id {
    egui::Id::new(("script-graph", at.slug()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScriptSessionId(u64);

pub(crate) struct ScriptSession {
    id: ScriptSessionId,
    at: ScriptRow,
}

#[derive(Default)]
pub struct ScriptViews {
    sessions: Vec<ScriptSession>,
    views: HashMap<ScriptSessionId, ScriptViewState>,
    pub keys: ScriptKeys,
    /// Where a script view was drawn the frame before, which the next frame's keys are taken for
    /// while the pointer is over it.
    pub shown_rect: Option<Rect>,
    pub run: Option<run::ScriptRun>,
    setups: HashMap<ScriptSessionId, run::RunSetup>,
    fire: Option<ScriptSessionId>,
    pending: Option<(ScriptSessionId, PendingScriptAction)>,
    pub callout: Option<ScriptCallout>,
}

impl ScriptViews {
    pub fn session(&mut self, at: &ScriptRow) -> ScriptSessionId {
        if let Some(id) = self.find(at) {
            return id;
        }
        let id = ScriptSessionId(self.sessions.len() as u64);
        self.sessions.push(ScriptSession { id, at: at.clone() });
        id
    }

    pub fn find(&self, at: &ScriptRow) -> Option<ScriptSessionId> {
        self.sessions
            .iter()
            .find(|session| session.at == *at)
            .map(|session| session.id)
    }

    pub fn relocate(&mut self, from: &ScriptRow, to: ScriptRow) {
        if let Some(session) = self.sessions.iter_mut().find(|session| session.at == *from) {
            session.at = to.clone();
        }
        if let Some(callout) = self.callout.as_mut().filter(|callout| callout.at == *from) {
            callout.at = to;
        }
    }

    pub fn view(&self, at: &ScriptRow) -> Option<&ScriptViewState> {
        self.views.get(&self.find(at)?)
    }

    pub fn view_mut(
        &mut self,
        at: &ScriptRow,
        user: &UserFolder,
        project: Option<&Project>,
    ) -> &mut ScriptViewState {
        let id = self.session(at);
        self.views.entry(id).or_insert_with(|| {
            let layout = project
                .map(|project| user.load_layout(&project.root, &at.slug()))
                .unwrap_or_default();
            ScriptViewState::opened(layout)
        })
    }

    pub(crate) fn run_of(&self, at: &ScriptRow) -> Option<&run::ScriptRun> {
        let id = self.find(at)?;
        self.run.as_ref().filter(|run| run.doc == id)
    }

    pub(crate) fn run_of_mut(&mut self, at: &ScriptRow) -> Option<&mut run::ScriptRun> {
        let id = self.find(at)?;
        self.run.as_mut().filter(|run| run.doc == id)
    }

    pub(crate) fn existing_view_mut(&mut self, at: &ScriptRow) -> Option<&mut ScriptViewState> {
        let id = self.find(at)?;
        self.views.get_mut(&id)
    }

    pub(crate) fn setup_of(&self, at: &ScriptRow) -> run::RunSetup {
        self.find(at)
            .and_then(|id| self.setups.get(&id))
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn set_setup(&mut self, at: &ScriptRow, setup: run::RunSetup) {
        let id = self.session(at);
        self.setups.insert(id, setup);
    }

    pub fn fire(&mut self, at: &ScriptRow) {
        self.fire = Some(self.session(at));
    }

    pub(crate) fn take_fire(&mut self, at: &ScriptRow) -> bool {
        let fired = self.fire.take();
        fired.is_some() && fired == self.find(at)
    }

    pub(crate) fn set_pending(&mut self, at: &ScriptRow, action: PendingScriptAction) {
        let id = self.session(at);
        self.pending = Some((id, action));
    }

    pub(crate) fn take_pending(&mut self, at: &ScriptRow) -> Option<PendingScriptAction> {
        let id = self.find(at)?;
        self.pending
            .take_if(|(pending, _)| *pending == id)
            .map(|(_, action)| action)
    }

    pub fn close(&mut self) {
        self.sessions.clear();
        self.views.clear();
        self.setups.clear();
        self.run = None;
        self.fire = None;
        self.pending = None;
    }
}

pub(crate) enum PendingScriptAction {
    /// Links the output of `from` a scene names by its label to `to`.
    Connect {
        from: String,
        output: String,
        to: String,
    },
}
