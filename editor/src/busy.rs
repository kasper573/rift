//! Work the editor waits on before it takes input again, done off the frame so the window never
//! stalls: the interface stays drawn but takes no input, and says what it waits on once the wait
//! outlasts [`SHOWN_AFTER`], so a short wait never flickers a dialog.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bevy::platform::cell::SyncCell;
use ui::egui::Context;

use crate::app::Editor;
use crate::project::Project;

/// The steps the editor waits on, in the order they finish.
#[derive(Default)]
pub struct Busy {
    steps: VecDeque<Step>,
    /// Since when the editor has been waiting without a break.
    since: Option<Instant>,
}

/// How far a step's work has come, as its work tells it.
#[derive(Default)]
pub struct Progress {
    doing: Mutex<String>,
    done: AtomicUsize,
    total: AtomicUsize,
}

impl Progress {
    /// Starts on `doing`, counting from nothing again, towards `total` when it is known.
    pub fn doing(&self, doing: impl Into<String>, total: Option<usize>) {
        *self.doing.lock().unwrap_or_else(PoisonError::into_inner) = doing.into();
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total.unwrap_or(0), Ordering::Relaxed);
    }

    /// Counts `done` of what the work is doing.
    pub fn done(&self, done: usize) {
        self.done.store(done, Ordering::Relaxed);
    }

    /// Counts one more of what the work is doing.
    pub fn one_more(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }

    fn line(&self) -> (String, Option<f32>) {
        let doing = self
            .doing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let done = self.done.load(Ordering::Relaxed);
        match self.total.load(Ordering::Relaxed) {
            0 if done == 0 => (doing, None),
            0 => (format!("{doing} · {}", ui::words::thousands(done)), None),
            total => (
                format!(
                    "{doing} · {} of {}",
                    ui::words::thousands(done.min(total)),
                    ui::words::thousands(total)
                ),
                Some(done.min(total) as f32 / total as f32),
            ),
        }
    }
}

impl Busy {
    pub fn is_busy(&self) -> bool {
        !self.steps.is_empty()
    }

    fn push(&mut self, step: Step) {
        self.since.get_or_insert_with(Instant::now);
        self.steps.push_back(step);
    }
}

impl Editor {
    /// Runs `work` off the frame on the background pool, then `then` with what it made, the
    /// editor waiting for both, after every step it already waits on. `work` starts only once
    /// those are done, so it starts from what they leave.
    pub fn run_busy<T: Send + 'static>(
        &mut self,
        label: impl Into<String>,
        work: impl FnOnce(&Progress) -> T + Send + 'static,
        then: impl FnOnce(&mut Editor, T) + Send + 'static,
    ) {
        self.busy.push(Step {
            label: label.into(),
            progress: Arc::new(Progress::default()),
            wait: SyncCell::new(Box::new(OffFrame {
                state: Some(OffFrameState::Waiting(Box::new(work))),
                then: Some(Box::new(then)),
            })),
        });
    }

    /// Runs `then` once the open folder's loads are in, so what it asks of the folder's usages and
    /// report is the files as they stand: at once when they are and the editor waits on nothing
    /// else, otherwise with the editor waiting for them.
    pub fn when_settled(
        &mut self,
        label: impl Into<String>,
        then: impl FnOnce(&mut Editor) + Send + 'static,
    ) {
        self.once(label, settled, "Waiting for the game to load", then);
    }

    /// Does `act` once the open folder's loads are in, as [`Editor::when_settled`] runs it: what
    /// it refuses is given back when it runs at once, and toasted when it has to wait.
    pub fn act_when_settled(
        &mut self,
        label: impl Into<String>,
        act: impl FnOnce(&mut Editor) -> Result<(), String> + Send + 'static,
    ) -> Result<(), String> {
        if !self.busy.is_busy() && settled(self) {
            return act(self);
        }
        self.when_settled(label, move |editor| {
            if let Err(refusal) = act(editor) {
                editor.toast(refusal);
            }
        });
        Ok(())
    }

    /// Runs `then` once every step the editor waits on is done: at once when it waits on none.
    pub fn when_idle(
        &mut self,
        label: impl Into<String>,
        then: impl FnOnce(&mut Editor) + Send + 'static,
    ) {
        self.once(label, |_| true, "", then);
    }

    /// Runs `then` once every step the editor waits on is done and `ready` holds, saying it is
    /// `waiting` until then: at once when it does and the editor waits on nothing else.
    fn once(
        &mut self,
        label: impl Into<String>,
        ready: fn(&Editor) -> bool,
        waiting: &str,
        then: impl FnOnce(&mut Editor) + Send + 'static,
    ) {
        if !self.busy.is_busy() && ready(self) {
            then(self);
            return;
        }
        let progress = Progress::default();
        progress.doing(waiting, None);
        self.busy.push(Step {
            label: label.into(),
            progress: Arc::new(progress),
            wait: SyncCell::new(Box::new(Once {
                ready,
                then: Some(Box::new(then)),
            })),
        });
    }

    /// Finishes every step whose wait is over, in order. A step's own steps, those it started as
    /// it finished, come before the steps that were waiting behind it.
    pub fn poll_busy(&mut self) {
        while let Some(mut step) = self.busy.steps.pop_front() {
            let behind = std::mem::take(&mut self.busy.steps);
            let finished = step.wait.get().poll(self, &step.progress);
            if !finished {
                self.busy.steps.push_front(step);
            }
            self.busy.steps.extend(behind);
            if !finished {
                return;
            }
        }
        self.busy.since = None;
    }

    /// Waits out every step the editor waits on, blocking: for what must hold before going on,
    /// such as a test's next step.
    pub fn finish_busy(&mut self) {
        while self.busy.is_busy() {
            if let Some(step) = self.busy.steps.front_mut() {
                step.wait.get().block(&mut self.project);
            }
            self.poll_busy();
        }
    }
}

/// Lays the busy veil over the interface while the editor waits: it takes every input, and once
/// the wait outlasts [`SHOWN_AFTER`], says what the editor waits on.
pub(crate) fn show(editor: &Editor, ctx: &Context) {
    let (Some(since), Some(step)) = (editor.busy.since, editor.busy.steps.front()) else {
        return;
    };
    ctx.request_repaint();
    let waited = since.elapsed();
    let shown = (waited >= SHOWN_AFTER).then(|| {
        let (line, fraction) = step.progress.line();
        ui::dialog::Waiting {
            title: &step.label,
            line,
            fraction,
        }
    });
    ui::dialog::veil(ctx, "busy", shown.as_ref());
}

/// How long the editor waits before saying what it waits on.
pub const SHOWN_AFTER: Duration = Duration::from_millis(300);

struct Step {
    label: String,
    progress: Arc<Progress>,
    // The editor is shared with bevy's systems, which never reach a step's wait but through it.
    wait: SyncCell<Box<dyn Wait + Send>>,
}

trait Wait {
    /// Finishes the step when what it waits on is done, saying whether it did.
    fn poll(&mut self, editor: &mut Editor, progress: &Arc<Progress>) -> bool;

    /// Blocks until what the step waits on is done.
    fn block(&mut self, project: &mut Option<Project>);
}

type Work<T> = Box<dyn FnOnce(&Progress) -> T + Send>;
type Then<T> = Box<dyn FnOnce(&mut Editor, T) + Send>;
type Act = Box<dyn FnOnce(&mut Editor) + Send>;

struct OffFrame<T> {
    state: Option<OffFrameState<T>>,
    then: Option<Then<T>>,
}

enum OffFrameState<T> {
    Waiting(Work<T>),
    Running(JoinHandle<T>),
    Made(T),
}

impl<T: Send + 'static> Wait for OffFrame<T> {
    fn poll(&mut self, editor: &mut Editor, progress: &Arc<Progress>) -> bool {
        let state = match self.state.take().expect("a step finishes once") {
            OffFrameState::Waiting(work) => {
                let progress = progress.clone();
                OffFrameState::Running(std::thread::spawn(move || {
                    crate::project::background().install(|| work(&progress))
                }))
            }
            OffFrameState::Running(running) if running.is_finished() => {
                OffFrameState::Made(joined(running))
            }
            state => state,
        };
        match state {
            OffFrameState::Made(made) => {
                (self.then.take().expect("a step finishes once"))(editor, made);
                true
            }
            state => {
                self.state = Some(state);
                false
            }
        }
    }

    fn block(&mut self, _: &mut Option<Project>) {
        self.state = self.state.take().map(|state| match state {
            OffFrameState::Waiting(work) => OffFrameState::Made(work(&Progress::default())),
            OffFrameState::Running(running) => OffFrameState::Made(joined(running)),
            made => made,
        });
    }
}

/// What the thread `running` made, or its panic, carried on here.
fn joined<T>(running: JoinHandle<T>) -> T {
    running
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

struct Once {
    ready: fn(&Editor) -> bool,
    then: Option<Act>,
}

impl Wait for Once {
    fn poll(&mut self, editor: &mut Editor, _: &Arc<Progress>) -> bool {
        if !(self.ready)(editor) {
            return false;
        }
        (self.then.take().expect("a step finishes once"))(editor);
        true
    }

    fn block(&mut self, project: &mut Option<Project>) {
        if let Some(project) = project {
            project.settle();
        }
    }
}

fn settled(editor: &Editor) -> bool {
    editor
        .project
        .as_ref()
        .is_none_or(|project| !project.loading())
}
