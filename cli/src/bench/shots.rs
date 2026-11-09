//! Pictures of what the client and the editor show, taken through their probes as the bench drives
//! them: the client's interface over the world at the moments a player sees most of it, and the
//! editor's stage from three sides, its tree with a selection and a form. Each run writes a set of
//! snapshots `rift compare` holds against another, as `rift snapshot` writes the world's.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use editor::scenes::SceneStep;
use engine::core::content::Content;
use engine::core::math::{Pos, Rect, Size};
use engine::core::units::{Degrees, Meters, Seconds, WindowPx};
use engine::systems::GameSettings;
use engine::systems::probe::ProbeSnapshot;
use engine::systems::staging::Staging;

use super::driver::Driver;
use super::editor::EditorDriver;
use super::failure_text;
use super::hands::{self, BUDGET, Failure};
use super::scenario::{CameraOp, Cond, Step};
use super::scenarios::ScenarioSlots;
use crate::compare::write_json;
use crate::render::VideoPreset;
use crate::snapshot::{MANIFEST, Manifest, PictureRect, TakenShot, hash_of};

/// The client's shots at each of `videos`, into the set `<out>/<adapter>/client`. Fails once every
/// shot is taken when any could not be.
pub fn client(
    game: &Path,
    content: &Content,
    slots: &ScenarioSlots,
    videos: &[VideoPreset],
    steps: &Path,
    out: &Path,
) -> Result<(), String> {
    let shots = client_shots(slots)?;
    let mut set: Option<Set> = None;
    let mut failed = Vec::new();
    let profiles = tempfile::tempdir().expect("a folder for the clients' profiles");
    for &video in videos {
        let profile = profiles.path().join(video.name());
        let mut driver = Driver::start(game, content, slots.character, video, &profile, steps)
            .map_err(|failure| failure_text(&failure))?;
        let adapter = driver
            .adapter()
            .map_err(|failure| failure_text(&failure))?
            .unwrap_or_default();
        let set = set.get_or_insert_with(|| Set::new(out, &adapter, CLIENT_SET));
        for shot in &shots {
            let relative = picture_path(video, &shot.name);
            let path = set.root.join(&relative);
            match take_client_shot(&mut driver, shot, &path) {
                Ok(masks) => set.add(&shot.name, video, relative, masks)?,
                Err(failure) => {
                    println!(
                        "[shots] client {} {}: {}",
                        video.name(),
                        shot.name,
                        failure_text(&failure)
                    );
                    failed.push(format!("{}/{}", video.name(), shot.name));
                    if matches!(failure, Failure::Crashed(_)) {
                        driver.end();
                        driver =
                            Driver::start(game, content, slots.character, video, &profile, steps)
                                .map_err(|failure| failure_text(&failure))?;
                    }
                }
            }
        }
    }
    set.map_or(Ok(()), |set| set.write())?;
    refused(failed)
}

/// The editor's shots at each of `videos`, into the set `<out>/<adapter>/editor`: the area the
/// game starts in as the editor frames it whole, from near above and from low aside, with what
/// stands nearest the middle selected, and the game's settings form.
pub fn editor(
    game: &Path,
    content: &Content,
    videos: &[VideoPreset],
    steps: &Path,
    out: &Path,
) -> Result<(), String> {
    let area = content
        .built::<GameSettings>()
        .initial_spawn
        .area
        .path(content)
        .to_owned();
    let mut set: Option<Set> = None;
    for &video in videos {
        let (row, adapter) = middle_row(game, &area, video, steps)?;
        let set = set.get_or_insert_with(|| Set::new(out, &adapter, EDITOR_SET));
        let shots: Vec<(&str, PathBuf)> = EDITOR_SHOTS
            .iter()
            .map(|&name| (name, picture_path(video, name)))
            .collect();
        let at = |name: &str| {
            let (_, relative) = shots
                .iter()
                .find(|(shot, _)| *shot == name)
                .expect("an editor shot by its name");
            SceneStep::Screenshot(set.root.join(relative))
        };
        let scene = vec![
            at(WHOLE),
            SceneStep::Camera {
                distance: NEAR.0,
                yaw: Some(NEAR.1),
                pitch: Some(NEAR.2),
            },
            at(NEAR_SHOT),
            SceneStep::Camera {
                distance: LOW.0,
                yaw: Some(LOW.1),
                pitch: Some(LOW.2),
            },
            at(LOW_SHOT),
            SceneStep::Select(vec![row]),
            at(SELECTED),
            SceneStep::Game,
            at(FORM),
        ];
        for (_, relative) in &shots {
            let _ = std::fs::remove_file(set.root.join(relative));
            if let Some(folder) = set.root.join(relative).parent() {
                std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
            }
        }
        let mut driver = EditorDriver::start(game, &area, &scene, video, steps)
            .map_err(|failure| failure_text(&failure))?;
        let last = set.root.join(&shots.last().expect("editor shots").1);
        let since = Instant::now();
        while !last.exists() {
            driver
                .snapshot()
                .map_err(|failure| failure_text(&failure))?;
            if since.elapsed() > SHOTS_WITHIN {
                return Err(format!(
                    "the editor took its shots no further than those before {}",
                    last.display()
                ));
            }
        }
        drop(driver);
        for (name, relative) in shots {
            set.add(name, video, relative, Vec::new())?;
        }
    }
    set.map_or(Ok(()), |set| set.write())
}

/// A shot of the client: the scene it is taken in, and what the picture waits for once it is set.
struct ClientShot {
    name: String,
    stage: Vec<Staging>,
    setup: Vec<Step>,
    ready: fn(&ProbeSnapshot) -> bool,
}

/// The view on the world from home with nothing open, each window the game's scenarios open, an
/// item's options and its card, a dialogue and a notice, wherever the game has what each needs.
fn client_shots(slots: &ScenarioSlots) -> Result<Vec<ClientShot>, String> {
    let home = slots
        .home()
        .ok_or("the game's scenarios have nowhere to stand")?;
    let at_home = vec![Staging::Place { place: home }];
    let homed = || vec![Step::Camera(CameraOp::Home), Step::Wait(Cond::Stood)];
    let shot = |name: &str, stage: Vec<Staging>, setup: Vec<Step>| ClientShot {
        name: name.to_owned(),
        stage,
        setup,
        ready: |_| true,
    };
    let mut shots = vec![shot(
        "hud",
        at_home.clone(),
        [homed(), vec![Step::Wait(Cond::Settled)]].concat(),
    )];
    for &action in &slots.windows {
        let name: &'static str = action.into();
        shots.push(shot(
            &format!("window-{name}"),
            at_home.clone(),
            [homed(), vec![Step::Key(action), Step::Wait(Cond::Settled)]].concat(),
        ));
    }
    if let Some(&first) = slots.windows.first() {
        let opened = || [homed(), vec![Step::Key(first), Step::Wait(Cond::Settled)]].concat();
        shots.push(shot(
            "item-options",
            at_home.clone(),
            [opened(), vec![Step::SlotOptions, Step::Wait(Cond::Settled)]].concat(),
        ));
        shots.push(shot(
            "item-card",
            at_home.clone(),
            [
                opened(),
                vec![
                    Step::HoverSlot,
                    Step::Wait(Cond::Seconds(CARD_SHOWS)),
                    Step::Wait(Cond::Settled),
                ],
            ]
            .concat(),
        ));
    }
    if let Some(talk) = &slots.talk_place {
        shots.push(ClientShot {
            ready: |snapshot| snapshot.stage.as_ref().is_some_and(|stage| !stage.typing),
            ..shot(
                "dialogue",
                vec![Staging::Place {
                    place: talk.place(),
                }],
                [
                    homed(),
                    vec![Step::Stage(Staging::Talk), Step::Wait(Cond::Settled)],
                ]
                .concat(),
            )
        });
    }
    if let Some(notification) = slots.notification {
        shots.push(ClientShot {
            ready: |snapshot| {
                let notices = &snapshot.notifications;
                !notices.captions.rows.is_empty()
                    || !notices.milestones.rows.is_empty()
                    || !notices.bubbles.is_empty()
                    || notices.intro.is_some()
            },
            ..shot(
                "notice",
                at_home.clone(),
                [homed(), vec![Step::Stage(Staging::Notify { notification })]].concat(),
            )
        });
    }
    Ok(shots)
}

/// Sets the shot's scene, waits for it to be ready and for the interface to come to rest, and
/// takes its picture at `path`: all but the interface masked, as the world beneath it lives on
/// from run to run (grass sways, clouds drift, everyone moves) and is the world's snapshots' to
/// hold.
fn take_client_shot(
    driver: &mut Driver,
    shot: &ClientShot,
    path: &Path,
) -> Result<Vec<PictureRect>, Failure> {
    driver.set_scene(&shot.name, &shot.stage, &shot.setup)?;
    let ready = shot.ready;
    hands::wait_for(driver, "the shot's moment", BUDGET, |snapshot| {
        ready(snapshot).then_some(())
    })?;
    driver.wait(&Cond::Settled)?;
    driver.wait(&Cond::Seconds(EASED))?;
    let snapshot = driver.snapshot()?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|error| Failure::Harness(format!("{}: {error}", folder.display())))?;
    }
    driver.picture(path)?;
    let (width, height) = image::image_dimensions(path)
        .map_err(|error| Failure::Harness(format!("{}: {error}", path.display())))?;
    let Some(screen) = snapshot.screen else {
        return Ok(Vec::new());
    };
    let scale = width as f32 / screen.width.max(1.0);
    let interface: Vec<Rect<WindowPx>> = snapshot
        .ui
        .iter()
        .filter(|element| !element.in_world)
        .map(|element| element.rect)
        .chain(snapshot.covered.iter().copied())
        .chain(snapshot.stage.as_ref().and_then(|stage| stage.rect))
        .collect();
    Ok(outside(&interface, scale, width, height))
}

/// The cells of a `width` by `height` picture, `scale` times the window's size, that no rect of
/// `kept` touches, as rects: each row's runs of such cells, joined with the rows below that run
/// alike.
fn outside(kept: &[Rect<WindowPx>], scale: f32, width: u32, height: u32) -> Vec<PictureRect> {
    let columns = width.div_ceil(CELL);
    let rows = height.div_ceil(CELL);
    let mut open: Vec<PictureRect> = Vec::new();
    let mut masks = Vec::new();
    for row in 0..rows {
        let y = row * CELL;
        let tall = CELL.min(height - y);
        let mut runs: Vec<PictureRect> = Vec::new();
        for column in 0..columns {
            let x = column * CELL;
            let wide = CELL.min(width - x);
            let cell = Rect::new(
                Pos::new(x as f32 / scale, y as f32 / scale),
                Size::new(wide as f32 / scale, tall as f32 / scale),
            );
            if kept.iter().any(|rect| rect.intersects(&cell)) {
                continue;
            }
            match runs.last_mut() {
                Some(run) if run.x + run.width == x => run.width += wide,
                _ => runs.push(PictureRect {
                    x,
                    y,
                    width: wide,
                    height: tall,
                }),
            }
        }
        let (mut grown, mut ended): (Vec<PictureRect>, Vec<PictureRect>) = (Vec::new(), Vec::new());
        for mut mask in open.drain(..) {
            match runs
                .iter()
                .position(|run| run.x == mask.x && run.width == mask.width)
            {
                Some(index) => {
                    runs.swap_remove(index);
                    mask.height += tall;
                    grown.push(mask);
                }
                None => ended.push(mask),
            }
        }
        masks.extend(ended);
        open = grown;
        open.extend(runs);
    }
    masks.extend(open);
    masks
}

/// The row of the scene tree of what the stage holds nearest its middle as the editor first frames
/// `area`, and the adapter the editor draws on.
fn middle_row(
    game: &Path,
    area: &str,
    video: VideoPreset,
    steps: &Path,
) -> Result<(String, String), String> {
    let mut driver = EditorDriver::start(game, area, &[], video, steps)
        .map_err(|failure| failure_text(&failure))?;
    driver.opened().map_err(|failure| failure_text(&failure))?;
    let adapter = driver.adapter().map_err(|failure| failure_text(&failure))?;
    let row = driver
        .stage()
        .map_err(|failure| failure_text(&failure))?
        .marks
        .into_iter()
        .next()
        .map(|mark| mark.row)
        .ok_or("the editor's stage shows nothing it holds to select")?;
    Ok((row, adapter))
}

/// A set of snapshots being taken: where it is, and what it holds so far.
struct Set {
    root: PathBuf,
    manifest: Manifest,
}

impl Set {
    fn new(out: &Path, adapter: &str, part: &str) -> Set {
        Set {
            root: out.join(adapter).join(part),
            manifest: Manifest {
                build: engine::core::content::version::build().to_owned(),
                adapter: adapter.to_owned(),
                shots: Vec::new(),
            },
        }
    }

    fn add(
        &mut self,
        name: &str,
        video: VideoPreset,
        relative: PathBuf,
        masks: Vec<PictureRect>,
    ) -> Result<(), String> {
        let path = self.root.join(&relative);
        let (width, height) = image::image_dimensions(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let (hash, _) = hash_of(&path)?;
        println!("[shots] {}/{name} taken", video.name());
        self.manifest.shots.push(TakenShot {
            name: name.to_owned(),
            video,
            path: relative,
            width,
            height,
            hash,
            frames: None,
            stood_after_frames: None,
            crashed: false,
            masks,
        });
        Ok(())
    }

    fn write(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.root).map_err(|error| error.to_string())?;
        write_json(&self.root.join(MANIFEST), &self.manifest)?;
        println!(
            "[shots] {} pictures by {} in {}",
            self.manifest.shots.len(),
            self.manifest.adapter,
            self.root.display()
        );
        Ok(())
    }
}

fn picture_path(video: VideoPreset, name: &str) -> PathBuf {
    Path::new(video.name()).join(format!("{name}.png"))
}

fn refused(failed: Vec<String>) -> Result<(), String> {
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("shots not taken: {}", failed.join(", ")))
    }
}

const CLIENT_SET: &str = "client";
const EDITOR_SET: &str = "editor";
const WHOLE: &str = "stage-whole";
const NEAR_SHOT: &str = "stage-near";
const LOW_SHOT: &str = "stage-low";
const SELECTED: &str = "tree-selected";
const FORM: &str = "form-game";
const EDITOR_SHOTS: [&str; 5] = [WHOLE, NEAR_SHOT, LOW_SHOT, SELECTED, FORM];
/// The stage seen from near above, and from low aside: how far off, turned and tilted.
const NEAR: (Meters, Degrees, Degrees) = (Meters(40.0), Degrees(0.0), Degrees(55.0));
const LOW: (Meters, Degrees, Degrees) = (Meters(90.0), Degrees(135.0), Degrees(20.0));
/// How long the pointer rests on an item for its card to show.
const CARD_SHOWS: Seconds = Seconds(1.0);
/// How long the interface is given to finish easing once it holds still: a bust lighting up as
/// its speaker talks, a window fading in.
const EASED: Seconds = Seconds(1.0);
/// The side of the square cells, in the picture's pixels, a picture is masked by.
const CELL: u32 = 8;
const SHOTS_WITHIN: Duration = Duration::from_secs(600);
