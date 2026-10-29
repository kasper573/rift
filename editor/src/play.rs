use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{Sender, TryRecvError, channel};
use std::time::{Duration, Instant};
use ui::chip;
use ui::dialog;
use ui::egui;
use ui::form;
use ui::sheet;
use ui::theme;
use ui::words;

use engine::core::content::{Content, ContentFile, ContentScope, version, with_content};
use engine::core::time::UtcHour;
use engine::systems::GameSettings;
use engine::systems::class::ClassDef;
use engine::systems::launch::{
    self, BUILD_INFO_FLAG, BuildInfo, CLIENT_ENV_PREFIX, ClientConfig, ReadyLine,
    SERVER_ENV_PREFIX, ServerConfig, ServerReady, TEST_CHARACTER_ROUTE, TestCharacterRequest,
    fake_token,
};
use engine::systems::player::SpawnPolicy;
use engine::systems::player::snapshot::{CharacterSnapshot, CharacterStart};
use engine::systems::script::Place;
use process_wrap::std::{ChildWrapper, CommandWrap};
use ui::controls::{self, Pill};
use ui::egui::{RichText, Ui};
use ui::glyph::{self, Mark};

use crate::app::{BottomTab, ChannelInbox, Editor, ModalDialog, OutputLine, content_version};
use crate::keymap::EditorKey;
use crate::project::{EditRefusal, Project, problem_place};
use crate::shell::{self, content_tree, widgets};
use ui::input;

const SERVER: &str = "game-server";
const CLIENT: &str = "desktop-client";
const READY_WITHIN: Duration = Duration::from_secs(30);
const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

#[derive(Default)]
pub struct PlayTest {
    pub running: Option<RunningPlayTest>,
    pub character: Option<String>,
    pub failure: Option<PlayFailure>,
    pub last_start: Option<Place>,
    pub binaries: Option<GameBinaries>,
    pub checking: Option<ChannelInbox<GameBinaries>>,
}

pub enum PlayFailure {
    SaveFailed,
    NotStarted(String),
    Stopped(String),
}

impl PlayFailure {
    pub fn reason(&self) -> String {
        match self {
            PlayFailure::SaveFailed => {
                "saving failed, so the server would read stale rows.".to_owned()
            }
            PlayFailure::NotStarted(reason) | PlayFailure::Stopped(reason) => reason.clone(),
        }
    }
}

pub struct RunningPlayTest {
    pub character: String,
    pub start: Option<Place>,
    pub history_at: usize,
    phase: Phase,
    started: Instant,
    server: Process,
    client: Option<Process>,
    lines: ChannelInbox<OutputLine>,
    sender: Sender<OutputLine>,
    ports: Option<ServerReady>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Starting,
    Playing,
    Stopping,
}

struct Process {
    child: Box<dyn ChildWrapper>,
    stdin: Option<ChildStdin>,
    exited: Option<i32>,
}

#[derive(Clone)]
pub struct GameBinaries {
    pub folder: PathBuf,
    pub server: Result<BuildInfo, String>,
    pub client: Result<BuildInfo, String>,
}

impl GameBinaries {
    /// Why a play-test can't run on them, if it can't: the client joins only a server of its own
    /// build, and both must read the rows this editor writes, which its content version tells.
    pub fn problem(&self, content_version: &str) -> Option<String> {
        let (server, client) = match (&self.server, &self.client) {
            (Err(error), _) => return Some(format!("{SERVER} is {error} in this folder.")),
            (_, Err(error)) => return Some(format!("{CLIENT} is {error} in this folder.")),
            (Ok(server), Ok(client)) => (server, client),
        };
        if server.build != client.build {
            return Some(format!("{SERVER} and {CLIENT} are different builds."));
        }
        [(SERVER, server), (CLIENT, client)]
            .into_iter()
            .find(|(_, info)| info.content_version != content_version)
            .map(|(name, _)| format!("{name} is of a different content version than the editor."))
    }

    fn builds_agree(&self) -> bool {
        matches!(
            (&self.server, &self.client),
            (Ok(server), Ok(client)) if server.build == client.build
        )
    }

    fn named(&self) -> [(&'static str, &Result<BuildInfo, String>); 2] {
        [(SERVER, &self.server), (CLIENT, &self.client)]
    }
}

struct SpawnJob {
    ports: ServerReady,
    character: String,
    start: Option<Place>,
    sender: Sender<OutputLine>,
}

pub struct TestCharacter {
    pub key: String,
    pub level: u32,
    pub area: String,
}

impl RunningPlayTest {
    pub fn stop(&mut self) {
        self.phase = Phase::Stopping;
        if let Some(client) = &mut self.client {
            let _ = client.child.start_kill();
        }
        let _ = self.server.child.start_kill();
    }

    fn playing(&self) -> bool {
        self.phase == Phase::Playing
    }
}

pub fn poll(editor: &mut Editor) {
    let Some(running) = editor.play.running.as_mut() else {
        return;
    };
    for line in running.lines.drain() {
        if line.tag == SERVER
            && running.ports.is_none()
            && let Some(ports) = ready_ports(&line.text)
        {
            running.ports = Some(ports);
        }
        editor.output.say(line.tag, line.text);
    }
    for process in [Some(&mut running.server), running.client.as_mut()]
        .into_iter()
        .flatten()
    {
        if process.exited.is_none()
            && let Ok(Some(status)) = process.child.try_wait()
        {
            process.exited = Some(status.code().unwrap_or(-1));
        }
    }
    let server_exit = running.server.exited;
    let client_exit = running.client.as_ref().and_then(|client| client.exited);
    let mut failure: Option<PlayFailure> = None;
    let mut done = false;
    let mut to_spawn: Option<SpawnJob> = None;
    match running.phase {
        Phase::Stopping => {
            if server_exit.is_some() && client_exit.is_some() == running.client.is_some() {
                done = true;
            }
        }
        Phase::Starting => {
            if let Some(code) = server_exit {
                failure = Some(PlayFailure::Stopped(format!(
                    "{SERVER} exited with code {code} before it was ready."
                )));
            } else if let Some(ports) = running.ports.filter(|_| running.client.is_none()) {
                to_spawn = Some(SpawnJob {
                    ports,
                    character: running.character.clone(),
                    start: running.start,
                    sender: running.sender.clone(),
                });
            } else if running.started.elapsed() > READY_WITHIN {
                failure = Some(PlayFailure::Stopped(format!(
                    "no ready line from {SERVER} in {} s.",
                    READY_WITHIN.as_secs()
                )));
            }
        }
        Phase::Playing => {
            if let Some(code) = client_exit {
                failure = Some(PlayFailure::Stopped(format!(
                    "{CLIENT} exited with code {code}. The server was stopped too."
                )));
            } else if let Some(code) = server_exit {
                failure = Some(PlayFailure::Stopped(format!(
                    "{SERVER} exited with code {code} during play. The client was stopped too."
                )));
            }
        }
    }
    if let Some(SpawnJob {
        ports,
        character,
        start,
        sender,
    }) = to_spawn
    {
        let content = editor
            .project
            .as_ref()
            .and_then(|project| project.content.clone());
        match hand_over(content.as_ref(), ports.http, &character, start) {
            Ok(()) => {
                editor.output.say(
                    "editor",
                    format!("test character {character} handed to the server, starting {CLIENT}"),
                );
                match spawn_client(editor, ports, &character, sender) {
                    Ok(process) => {
                        if let Some(running) = editor.play.running.as_mut() {
                            running.client = Some(process);
                            running.phase = Phase::Playing;
                        }
                    }
                    Err(error) => {
                        failure = Some(PlayFailure::Stopped(format!(
                            "{CLIENT} did not start: {error}"
                        )))
                    }
                }
            }
            Err(error) => {
                failure = Some(PlayFailure::Stopped(format!(
                    "the server refused the test character: {error}"
                )))
            }
        }
    }
    if let Some(failure) = failure {
        fail(editor, failure);
    }
    if done && let Some(running) = editor.play.running.take() {
        for line in running.lines.drain() {
            editor.output.say(line.tag, line.text);
        }
        editor.output.say(
            "editor",
            format!(
                "stopped {SERVER} (it exited with code {})",
                server_exit.unwrap_or(-1)
            ),
        );
        if let Some(code) = client_exit {
            editor.output.say(
                "editor",
                format!("stopped {CLIENT} (it exited with code {code})"),
            );
        }
    }
}

pub fn start(editor: &mut Editor, start: Option<Place>) {
    if editor.project.is_none() {
        return;
    }
    if editor.play.running.is_some() {
        editor.toast("A play-test is running: Restart applies your edits, Stop ends it");
        return;
    }
    let Some(character) = current_character(editor) else {
        editor.toast(format!(
            "No test character: {} makes one",
            EditorKey::PlayedCharacter.name()
        ));
        return;
    };
    let Some(binaries) =
        check_binaries(editor).filter(|binaries| binaries.problem(content_version()).is_none())
    else {
        editor.dialogs.push(ModalDialog::Binaries);
        return;
    };
    editor.play.failure = None;
    if editor.has_unsaved() && !editor.save() {
        fail(editor, PlayFailure::SaveFailed);
        return;
    }
    if let Some(report) = editor.project.as_ref().and_then(Project::unplayable) {
        for error in &report.errors {
            editor.output.say(
                "editor",
                format!(
                    "{}: {}",
                    problem_place(&error.file, error.at.pointer()),
                    error.problem
                ),
            );
        }
        editor.panels.bottom = BottomTab::Output;
        fail(
            editor,
            PlayFailure::NotStarted(format!(
                "the game isn't whole yet: {}",
                EditRefusal::of(&report).sentence()
            )),
        );
        return;
    }
    editor.play.binaries = Some(binaries.clone());
    let folder = editor_folder(editor);
    let (sender, lines) = channel();
    editor.output.say(
        "editor",
        format!("starting {}", binaries.folder.join(SERVER).display()),
    );
    let server = match spawn_server(&binaries.folder, &folder, sender.clone()) {
        Ok(process) => process,
        Err(error) => {
            fail(
                editor,
                PlayFailure::NotStarted(format!("{SERVER} did not start: {error}")),
            );
            return;
        }
    };
    editor.play.character = Some(character.clone());
    editor.settings.last_play = Some(character.clone());
    editor.play.last_start = start;
    editor.play.running = Some(RunningPlayTest {
        character,
        start,
        history_at: editor
            .project
            .as_ref()
            .map_or(0, |project| project.history.applied()),
        phase: Phase::Starting,
        started: Instant::now(),
        server,
        client: None,
        lines: ChannelInbox::new(lines),
        sender,
        ports: None,
    });
    editor.panels.bottom = BottomTab::Output;
}

pub fn stop(editor: &mut Editor) {
    if let Some(running) = editor.play.running.as_mut() {
        editor.output.say("editor", "stopping the play-test");
        running.stop();
    }
}

pub fn restart(editor: &mut Editor) {
    let from = editor.play.last_start;
    if let Some(mut running) = editor.play.running.take() {
        running.stop();
        let _ = running.server.child.wait();
        if let Some(client) = running.client.as_mut() {
            let _ = client.child.wait();
        }
        for line in running.lines.drain() {
            editor.output.say(line.tag, line.text);
        }
        editor.output.say("editor", "restarting the play-test");
    }
    start(editor, from);
}

pub fn top_bar(editor: &mut Editor, ui: &mut Ui) {
    let characters = test_characters(editor);
    let current = current_character(editor);
    let (play, arrow) = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            let arrow = glyph::button(ui, Mark::CaretDown, 14.0, theme::TEXT, "Test character")
                .on_hover_text("Pick the test character");
            let play = controls::marked_primary(ui, Mark::Play, "Play").on_hover_text(format!(
                "Save, then play as the picked test character from its own start ({})",
                EditorKey::PlayFromStart.name()
            ));
            (play, arrow)
        })
        .inner;
    let menu = egui::Popup::from_toggle_button_response(&arrow)
        .id(ui.id().with("play-menu"))
        .width(230.0);
    let mut pick: Option<String> = None;
    let mut edit = false;
    let mut stop_now = false;
    let mut play_now = false;
    menu.show(|ui| {
        sheet::section(ui, "Test character");
        for character in &characters {
            let picked = current.as_deref() == Some(character.key.as_str());
            let response = ui.add(
                egui::Button::new(RichText::new(&character.key).size(13.0))
                    .selected(picked)
                    .frame(false),
            );
            ui.label(
                theme::faint(format!("level {} · {}", character.level, character.area)).size(11.5),
            );
            if response.clicked() {
                pick = Some(character.key.clone());
                play_now = true;
            }
        }
        if characters.is_empty() {
            ui.label(theme::muted("no test_character rows").size(12.5));
        }
        ui.separator();
        let label = if characters.is_empty() {
            "New test character"
        } else {
            "Edit test characters…"
        };
        if ui
            .add(widgets::shortcut_button(label, EditorKey::PlayedCharacter).frame(false))
            .clicked()
        {
            edit = true;
        }
        if ui
            .add_enabled(
                editor.play.running.is_some(),
                widgets::shortcut_button("Stop", EditorKey::StopPlay).frame(false),
            )
            .clicked()
        {
            stop_now = true;
        }
    });
    if let Some(key) = pick {
        editor.play.character = Some(key.clone());
        editor.settings.last_play = Some(key);
    }
    if play.clicked() || play_now {
        start(editor, None);
    }
    if edit {
        edit_test_characters(editor);
    }
    if stop_now {
        stop(editor);
    }
    let Some(running) = editor.play.running.as_ref() else {
        return;
    };
    let changes = editor.project.as_ref().map_or(0, |project| {
        project.history.applied().saturating_sub(running.history_at)
    });
    let playing = running.playing();
    let stopping = running.phase == Phase::Stopping;
    let mut restart_now = false;
    if controls::marked_danger(ui, Mark::Stop, "Stop").clicked() {
        stop_now = true;
    }
    if controls::keyed(ui, "Restart", None, !stopping)
        .on_hover_text("Save, then start both again as the same character")
        .clicked()
    {
        restart_now = true;
    }
    chip::plain(
        ui,
        format!("{changes} change{} since start", words::plural(changes)),
    );
    controls::pill(
        ui,
        if stopping {
            "stopping"
        } else if playing {
            "playing"
        } else {
            "starting"
        },
        if playing { Pill::Verdigris } else { Pill::Gold },
    );
    if stop_now {
        stop(editor);
    }
    if restart_now {
        restart(editor);
    }
}

pub fn banner(editor: &mut Editor, ui: &mut Ui) {
    let Some(failure) = editor.play.failure.as_ref() else {
        return;
    };
    let reason = failure.reason();
    let mut copy = false;
    let mut save = false;
    let mut again = false;
    let mut dismiss = false;
    egui::Area::new(ui.id().with("play-failure"))
        .order(egui::Order::Foreground)
        .fixed_pos(ui.max_rect().left_top() + egui::vec2(12.0, 12.0))
        .show(ui.ctx(), |ui| {
            ui::slip::show(ui, |ui| {
                ui.set_max_width(700.0);
                ui.horizontal_top(|ui| {
                    glyph::show(ui, Mark::Warning, 15.0, theme::DANGER);
                    ui.label(RichText::new("The play-test stopped.").strong().size(14.0));
                    ui.add(egui::Label::new(theme::muted(&reason).size(13.0)).wrap());
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if controls::primary(ui, "Copy output").clicked() {
                        copy = true;
                    }
                    if ui.button("Save to file…").clicked() {
                        save = true;
                    }
                    if controls::marked(ui, Mark::Play, "Play again").clicked() {
                        again = true;
                    }
                    if glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                        .on_hover_text("Hide")
                        .clicked()
                    {
                        dismiss = true;
                    }
                });
            });
        });
    if copy {
        let text = editor.output.text();
        ui.ctx().copy_text(text);
    }
    if save {
        save_output(editor);
    }
    if again {
        editor.play.failure = None;
        start(editor, editor.play.last_start);
    }
    if dismiss {
        editor.play.failure = None;
    }
}

pub fn save_output(editor: &Editor) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Save the output")
        .set_file_name("rift-editor-output.txt")
        .save_file()
    {
        let _ = std::fs::write(path, editor.output.text());
    }
}

pub fn status(editor: &mut Editor) {
    if let Some(running) = editor.play.running.as_ref() {
        let text = match running.phase {
            Phase::Starting => format!("play-test starting as {}…", running.character),
            Phase::Playing => format!("play-test playing as {}", running.character),
            Phase::Stopping => "play-test stopping…".to_owned(),
        };
        editor.status.right.push(text);
    } else if let Some(failure) = editor.play.failure.as_ref() {
        let reason = failure.reason();
        editor.status.right.push(format!(
            "play-test stopped · {}",
            reason.split('.').next().unwrap_or("")
        ));
    }
}

pub fn test_characters(editor: &Editor) -> Vec<TestCharacter> {
    let Some(project) = editor.project.as_ref() else {
        return Vec::new();
    };
    let content = project.content.clone();
    let reader = project.reader();
    let spawn_area = content.as_ref().map(|content| {
        content
            .built::<GameSettings>()
            .initial_spawn
            .area
            .path(content)
            .to_owned()
    });
    let mut characters: Vec<TestCharacter> = project
        .files
        .paths_of(CharacterSnapshot::TABLE)
        .iter()
        .filter_map(|file| {
            let key = file.as_str();
            let snapshot = project.read::<CharacterSnapshot>(key).ok()?;
            let level = content
                .as_ref()
                .and_then(|content| {
                    let class = reader.path_of(snapshot.class)?;
                    let class = content.by_path::<ClassDef>(class)?;
                    Some(class.get(content).level_at(snapshot.xp))
                })
                .unwrap_or(0);
            let area = match &snapshot.start {
                CharacterStart::At(place) => reader.path_of(place.area).unwrap_or("?").to_owned(),
                CharacterStart::InitialSpawn(_) => spawn_area
                    .clone()
                    .map(|area| format!("the initial spawn · {area}"))
                    .unwrap_or_else(|| "the initial spawn".to_owned()),
            };
            Some(TestCharacter {
                key: key.to_owned(),
                level,
                area,
            })
        })
        .collect();
    characters.sort_by(|a, b| a.key.cmp(&b.key));
    characters
}

/// Opens the picked test character, or makes one in the game folder when there is none.
pub fn edit_test_characters(editor: &mut Editor) {
    match current_character(editor) {
        Some(key) => editor.open_row(CharacterSnapshot::TABLE, &key),
        None => content_tree::new_file(editor, "", CharacterSnapshot::TABLE),
    }
}

pub fn current_character(editor: &Editor) -> Option<String> {
    let characters = test_characters(editor);
    editor
        .play
        .character
        .clone()
        .or_else(|| editor.settings.last_play.clone())
        .filter(|key| characters.iter().any(|character| character.key == *key))
        .or_else(|| characters.first().map(|character| character.key.clone()))
}

pub fn binaries_folder(editor: &Editor) -> PathBuf {
    editor
        .settings
        .binaries
        .clone()
        .unwrap_or_else(next_to_editor)
}

pub fn next_to_editor() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

// Starting a game binary for its build line takes long enough to feel as a freeze, so the check
// runs on a thread and answers `None` until it is done.
pub fn check_binaries(editor: &mut Editor) -> Option<GameBinaries> {
    let folder = binaries_folder(editor);
    if let Some(known) = editor
        .play
        .binaries
        .as_ref()
        .filter(|known| known.folder == folder)
    {
        return Some(known.clone());
    }
    if let Some(receiver) = &editor.play.checking {
        match receiver.try_recv() {
            Ok(binaries) if binaries.folder == folder => {
                editor.play.binaries = Some(binaries.clone());
                editor.play.checking = None;
                return Some(binaries);
            }
            Ok(_) | Err(TryRecvError::Disconnected) => editor.play.checking = None,
            Err(TryRecvError::Empty) => return None,
        }
    }
    let (sender, receiver) = channel();
    std::thread::spawn(move || {
        let _ = sender.send(GameBinaries {
            server: build_info(&folder.join(SERVER)),
            client: build_info(&folder.join(CLIENT)),
            folder,
        });
    });
    editor.play.checking = Some(ChannelInbox::new(receiver));
    None
}

fn build_info(binary: &Path) -> Result<BuildInfo, String> {
    if !binary.exists() {
        return Err("missing".to_owned());
    }
    let output = Command::new(binary)
        .arg(BUILD_INFO_FLAG)
        .output()
        .map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(text.trim()).map_err(|_| "no build line".to_owned())
}

pub fn binaries_body(editor: &mut Editor, ui: &mut Ui) -> bool {
    let checked = check_binaries(editor);
    let folder = binaries_folder(editor);
    let build = version::build();
    let content_version = content_version();
    let open_folder = editor.project.as_ref().map(|project| {
        project
            .content
            .as_ref()
            .map(|content| content.built::<GameSettings>().content_version.0.clone())
    });
    let problem = checked
        .as_ref()
        .and_then(|binaries| binaries.problem(content_version));
    let ok = checked.is_some() && problem.is_none();
    if checked.is_none() {
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }
    let mut close = false;
    let mut pick = false;
    let mut next_to = false;
    ui.horizontal(|ui| {
        if ok {
            glyph::show(ui, Mark::Settings, 18.0, theme::TEXT);
        } else {
            glyph::show(ui, Mark::Warning, 18.0, theme::WARN);
        }
        dialog::title(ui, "Game binaries");
    });
    ui.add_space(8.0);
    form::kv(ui, "folder", |ui| {
        let response = input::boxed(ui, Some("pick…"), |ui| {
            ui.add(
                egui::Label::new(theme::mono(shell::pretty_path(&folder)).size(13.0))
                    .sense(egui::Sense::click()),
            )
        });
        if response.clicked() {
            pick = true;
        }
    });
    ui.add_space(4.0);
    let row = |ui: &mut Ui, name: &str, value: &str, state: Option<bool>| {
        ui.horizontal(|ui| {
            ui.add_sized(
                egui::vec2(150.0, 20.0),
                egui::Label::new(theme::mono(name).size(13.0)),
            );
            ui.label(
                theme::mono(value)
                    .size(13.0)
                    .color(if state == Some(false) {
                        theme::DANGER
                    } else {
                        theme::TEXT
                    }),
            );
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match state {
                    Some(true) => {
                        glyph::show(ui, Mark::Check, 14.0, theme::OK);
                    }
                    Some(false) => {
                        glyph::show(ui, Mark::Cross, 14.0, theme::DANGER);
                    }
                    None => {}
                },
            );
        });
    };
    row(
        ui,
        "editor",
        &format!("build {build}, content version {content_version}"),
        None,
    );
    match &checked {
        Some(binaries) => {
            for (name, info) in binaries.named() {
                match info {
                    Ok(info) => row(
                        ui,
                        name,
                        &format!(
                            "build {}, content version {}",
                            info.build, info.content_version
                        ),
                        Some(binaries.builds_agree() && info.content_version == content_version),
                    ),
                    Err(error) => row(ui, name, error, Some(false)),
                }
            }
        }
        None => {
            for name in [SERVER, CLIENT] {
                row(ui, name, "checking…", None);
            }
        }
    }
    if let Some(version) = &open_folder {
        match version {
            Some(version) => row(
                ui,
                "open folder",
                &format!("content version {version}"),
                Some(version == content_version),
            ),
            None => row(ui, "open folder", "does not load", None),
        }
    }
    ui.add_space(8.0);
    if let Some(problem) = problem {
        ui.horizontal_wrapped(|ui| {
            glyph::show(ui, Mark::Warning, 14.0, theme::DANGER);
            ui.label(RichText::new(problem).color(theme::DANGER).size(13.0));
        });
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ok || checked.is_none() {
            if controls::primary(ui, "Done").clicked() {
                close = true;
            }
        } else {
            if ui.button("Choose another folder").clicked() {
                pick = true;
            }
            if ui.button("Use the ones next to the editor").clicked() {
                next_to = true;
            }
            if ui
                .add(egui::Button::new(theme::muted("Close")).frame(false))
                .clicked()
            {
                close = true;
            }
        }
    });
    if pick
        && let Some(folder) = rfd::FileDialog::new()
            .set_title("The folder holding game-server and desktop-client")
            .set_directory(&folder)
            .pick_folder()
    {
        editor.settings.binaries = Some(folder);
        editor.play.binaries = None;
    }
    if next_to {
        editor.settings.binaries = None;
        editor.play.binaries = None;
    }
    close
}

fn editor_folder(editor: &Editor) -> PathBuf {
    editor
        .project
        .as_ref()
        .map(|project| project.root.clone())
        .unwrap_or_default()
}

fn spawn_server(binaries: &Path, folder: &Path, sender: Sender<OutputLine>) -> io::Result<Process> {
    let config = ServerConfig {
        bind: LOCALHOST,
        port: 0,
        ws_port: 0,
        max_clients: 8,
        content_dir: folder.to_path_buf(),
        player_spawn_policy: SpawnPolicy::Map,
        player_immortal: false,
        rng_seed: None,
        daily_reset_utc_hour: UtcHour::MIDNIGHT,
        auth: launch::AuthMode::Fake,
        test_characters: true,
        exit_on_stdin_eof: true,
    };
    let mut command = Command::new(binaries.join(SERVER));
    command.envs(launch::env_of(SERVER_ENV_PREFIX, &config));
    spawn(command, SERVER, sender)
}

fn spawn_client(
    editor: &Editor,
    ports: ServerReady,
    character: &str,
    sender: Sender<OutputLine>,
) -> io::Result<Process> {
    let binaries = binaries_folder(editor);
    let profile = editor.user.profiles_dir().join("play-test");
    std::fs::create_dir_all(&profile)?;
    let config = ClientConfig {
        game_server_url: format!("http://{LOCALHOST}:{}", ports.http),
        game_server_ws_url: format!("ws://{LOCALHOST}:{}", ports.ws),
        auth: launch::AuthMode::Fake,
        access_token: Some(fake_token(character)),
        profile_dir: profile,
        exit_on_stdin_eof: true,
        probe: None,
    };
    let mut command = Command::new(binaries.join(CLIENT));
    command.envs(launch::env_of(CLIENT_ENV_PREFIX, &config));
    spawn(command, CLIENT, sender)
}

fn spawn(
    mut command: Command,
    tag: &'static str,
    sender: Sender<OutputLine>,
) -> io::Result<Process> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut wrap = CommandWrap::from(command);
    as_one(&mut wrap);
    let mut child = wrap.spawn()?;
    let stdin = child.stdin().take();
    if let Some(stdout) = child.stdout().take() {
        forward(stdout, tag, sender.clone());
    }
    if let Some(stderr) = child.stderr().take() {
        forward(stderr, tag, sender);
    }
    Ok(Process {
        child,
        stdin,
        exited: None,
    })
}

fn forward(output: impl Read + Send + 'static, tag: &'static str, sender: Sender<OutputLine>) {
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            let _ = sender.send(OutputLine {
                tag,
                text: plain(&line),
            });
        }
    });
}

fn ready_ports(line: &str) -> Option<ServerReady> {
    serde_json::from_str::<ReadyLine>(line.trim())
        .ok()
        .map(|line| line.ready)
}

fn hand_over(
    content: Option<&Content>,
    http: u16,
    character: &str,
    start: Option<Place>,
) -> Result<(), String> {
    let request = TestCharacterRequest {
        test_character: character.to_owned(),
        start: start.filter(|_| content.is_some()),
    };
    let body = match content {
        Some(content) => {
            with_content(ContentScope::of(content), || {
                serde_json::to_string(&request)
            })
            .0
        }
        None => serde_json::to_string(&request),
    }
    .map_err(|error| error.to_string())?;
    let response = ureq::post(format!("http://{LOCALHOST}:{http}{TEST_CHARACTER_ROUTE}"))
        .header("Authorization", fake_token(character))
        .send(body.as_bytes());
    match response {
        Ok(_) => Ok(()),
        Err(ureq::Error::StatusCode(code)) => Err(format!("status {code}")),
        Err(error) => Err(error.to_string()),
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = stdin.flush();
        }
        let _ = self.child.start_kill();
    }
}

// So that killing the process kills whatever it started too. process-wrap leaves the platform to its
// user: POSIX groups processes in a process group, Windows in a job object.
#[cfg(unix)]
fn as_one(wrap: &mut CommandWrap) {
    wrap.wrap(process_wrap::std::ProcessGroup::leader());
}

#[cfg(windows)]
fn as_one(wrap: &mut CommandWrap) {
    wrap.wrap(process_wrap::std::JobObject);
}

fn plain(line: &str) -> String {
    String::from_utf8_lossy(&strip_ansi_escapes::strip(line)).into_owned()
}

fn fail(editor: &mut Editor, failure: PlayFailure) {
    let stopped = matches!(failure, PlayFailure::Stopped(_));
    match &failure {
        PlayFailure::SaveFailed => {}
        PlayFailure::NotStarted(reason) => editor.output.say("editor", reason.clone()),
        PlayFailure::Stopped(reason) => {
            editor
                .output
                .say("editor", format!("play-test stopped: {reason}"));
            if let Some(running) = editor.play.running.as_mut() {
                running.stop();
            }
        }
    }
    editor.play.failure = Some(failure);
    if stopped {
        editor.toast("The play-test stopped; see Output");
    }
}
