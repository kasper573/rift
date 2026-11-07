//! Every shot of a game drawn into a set of pictures to compare builds by. A shot file holds one
//! shot, or a shot and a matrix of axes whose cross product it is drawn over: places, pitches,
//! yaws, zooms, hours, spells and phases. Each picture is drawn by a `rift render` process of its
//! own, since a renderer is not reusable and one crash must not take the run down, into a folder
//! by the adapter that drew it and the video preset, beside a manifest of what was drawn.

use std::path::{Path, PathBuf};
use std::process::Command;

use engine::core::render::camera::Zoom;
use engine::core::units::{Degrees, Meters};
use serde::{Deserialize, Serialize};

use crate::compare::{self, Allowance, Metrics, Thresholds, matches, read_json, write_json};
use crate::render::{
    CameraShot, RenderOutcome, Shot, Spot, VideoChoice, VideoFilter, VideoPreset, ViewSpec,
};

pub const MANIFEST: &str = "manifest.json";
pub const THRESHOLDS: &str = "thresholds.json";
/// How many times the noise between runs a shot's allowance is.
const NOISE_TIMES: f64 = 3.0;

/// What a set of snapshots holds: the build and adapter that drew it, and each picture.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Manifest {
    pub build: String,
    pub adapter: String,
    pub shots: Vec<TakenShot>,
}

impl Manifest {
    pub fn read(set: &Path) -> Result<Manifest, String> {
        read_json(&set.join(MANIFEST))
    }
}

/// A picture of the set: the shot's name and preset, where the picture is under the set, how big
/// it is, the hash of its bytes, how many frames it has if it is an arrival, after how many of
/// them the area stood whole, whether its render crashed instead, and the parts of it that move
/// on their own from one taking to the next, which no comparison holds against another.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TakenShot {
    pub name: String,
    pub video: VideoPreset,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub hash: String,
    pub frames: Option<usize>,
    pub stood_after_frames: Option<usize>,
    pub crashed: bool,
    #[serde(default)]
    pub masks: Vec<PictureRect>,
}

/// A rect of a picture, in its pixels.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PictureRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl TakenShot {
    /// What the picture goes by across sets: its preset and its shot's name.
    pub fn key(&self) -> String {
        format!("{}/{}", self.video.name(), self.name)
    }
}

/// A shot file: a shot, the axes it is drawn over, and the presets it is drawn at.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShotFile {
    shot: Shot,
    #[serde(default)]
    matrix: Matrix,
    #[serde(default = "both")]
    presets: Vec<VideoPreset>,
}

fn both() -> Vec<VideoPreset> {
    VideoPreset::BOTH.to_vec()
}

/// The axes a shot is drawn over, each with its values; one picture per combination. The places,
/// pitches, yaws and zooms frame the world's camera, so they need a shot whose view is it.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct Matrix {
    at: Vec<NamedSpot>,
    pitch: Vec<Degrees>,
    yaw: Vec<Degrees>,
    zoom: Vec<Zoom>,
    hour: Vec<HourAxis>,
    spell: Vec<String>,
    spell_phase: Vec<f32>,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct NamedSpot {
    name: String,
    x: Meters,
    y: Meters,
    height: Meters,
}

/// An hour along the hour axis: `"fixed"` for the area's light as authored, else the hour.
#[derive(Deserialize, Clone, Copy)]
#[serde(untagged)]
enum HourAxis {
    Fixed(FixedHour),
    At(f32),
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum FixedHour {
    Fixed,
}

/// Draws every shot under `shots` into `out`, at the presets `video` lets through, only those
/// whose names match `only` when given.
pub fn run(
    game: &Path,
    shots: &Path,
    out: &Path,
    video: VideoFilter,
    only: Option<&str>,
) -> Result<(), String> {
    let expanded = expand_all(shots)?;
    let game = std::path::absolute(game).map_err(|error| error.to_string())?;
    let out = std::path::absolute(out).map_err(|error| error.to_string())?;
    let scratch = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut manifest = Manifest {
        build: engine::core::content::version::build().to_owned(),
        adapter: String::new(),
        shots: Vec::new(),
    };
    let mut taken = 0;
    for (name, shot, presets) in &expanded {
        if only.is_some_and(|only| !matches(only, name)) {
            continue;
        }
        for &preset in presets {
            if !video.takes(preset) {
                continue;
            }
            taken += 1;
            let drawn = draw(
                &game,
                scratch.path(),
                &out,
                name,
                shot,
                preset,
                &manifest.adapter,
            )?;
            if manifest.adapter.is_empty() {
                manifest.adapter = drawn.adapter.clone();
            }
            println!(
                "[snapshot] {:<60} {}",
                drawn.shot.key(),
                if drawn.shot.crashed {
                    "CRASHED".to_owned()
                } else {
                    match drawn.shot.stood_after_frames {
                        Some(frames) => format!("drawn, stood after {frames} frames"),
                        None => "drawn".to_owned(),
                    }
                }
            );
            manifest.shots.push(drawn.shot);
        }
    }
    if taken == 0 {
        return Err(format!("no shot under {} to draw", shots.display()));
    }
    let set = out.join(&manifest.adapter);
    std::fs::create_dir_all(&set).map_err(|error| error.to_string())?;
    write_json(&set.join(MANIFEST), &manifest)?;
    println!(
        "[snapshot] {} pictures by {} in {}",
        manifest.shots.len(),
        manifest.adapter,
        set.display()
    );
    Ok(())
}

/// Draws every shot `runs` times, a set each, and writes each shot's allowance, three times the
/// worst noise between its runs and at least the floor, to `out/thresholds.json`.
pub fn noise(game: &Path, shots: &Path, out: &Path, runs: usize) -> Result<(), String> {
    if runs < 2 {
        return Err("the noise shows between at least two runs".to_owned());
    }
    let sets: Vec<PathBuf> = (0..runs)
        .map(|run| out.join(format!("run-{run}")))
        .collect();
    for set in &sets {
        run(game, shots, set, VideoFilter::Both, None)?;
    }
    let adapter = adapter_of(&sets[0])?;
    let sets: Vec<PathBuf> = sets.iter().map(|set| set.join(&adapter)).collect();
    noise_between(&sets, out)
}

/// Writes each shot's allowance, three times the worst noise between the sets taken of it and at
/// least the floor, to `out/thresholds.json`. Each set is the folder of one adapter's pictures, as
/// `rift compare` takes them.
pub fn noise_between(sets: &[PathBuf], out: &Path) -> Result<(), String> {
    let [first_set, others @ ..] = sets else {
        return Err("the noise shows between at least two sets".to_owned());
    };
    if others.is_empty() {
        return Err("the noise shows between at least two sets".to_owned());
    }
    let first = Manifest::read(first_set)?;
    let others: Vec<(&PathBuf, Manifest)> = others
        .iter()
        .map(|set| Manifest::read(set).map(|manifest| (set, manifest)))
        .collect::<Result<_, _>>()?;
    let mut thresholds = Thresholds::default();
    for taken in &first.shots {
        let key = taken.key();
        if taken.crashed {
            return Err(format!("{key} crashed: no noise to measure"));
        }
        let mut worst = Metrics {
            mssim: 1.0,
            worst_tile: 1.0,
            delta_e_share: 0.0,
        };
        let mut popped = compare::pop_in(first_set, taken)?;
        for (set, manifest) in &others {
            let again = manifest
                .shots
                .iter()
                .find(|shot| shot.key() == key)
                .ok_or_else(|| format!("{key} missing from {}", set.display()))?;
            if again.crashed {
                return Err(format!("{key} crashed: no noise to measure"));
            }
            let (a, b) = compare::compared_pictures((first_set, taken), (set, again))?;
            worst = worst.worst(compare::measure(&a, &b));
            popped = match (popped, compare::pop_in(set, again)?) {
                (Some(worst), Some(popped)) => Some(worst.max(popped)),
                (worst, popped) => worst.or(popped),
            };
        }
        let allowance = Allowance::of_noise(&worst, popped, NOISE_TIMES);
        println!(
            "[noise] {key:<60} structure {:.5} worst tile {:.5} colour shift {:.3}% popped {:.3}% → allowance {:.5} / {:.5} / {:.3}% / {:.3}%",
            worst.mssim,
            worst.worst_tile,
            worst.delta_e_share * 100.0,
            popped.unwrap_or_default() * 100.0,
            allowance.mssim_loss,
            allowance.tile_loss,
            allowance.delta_e_share * 100.0,
            allowance.pop_share * 100.0
        );
        thresholds.shots.insert(key, allowance);
    }
    std::fs::create_dir_all(out).map_err(|error| format!("{}: {error}", out.display()))?;
    write_json(&out.join(THRESHOLDS), &thresholds)?;
    println!("[noise] thresholds in {}", out.join(THRESHOLDS).display());
    Ok(())
}

/// The one adapter folder a set was drawn into.
fn adapter_of(set: &Path) -> Result<String, String> {
    let mut adapters: Vec<String> = std::fs::read_dir(set)
        .map_err(|error| format!("{}: {error}", set.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    adapters.sort();
    match adapters.as_slice() {
        [adapter] => Ok(adapter.clone()),
        [] => Err(format!("{}: nothing drawn", set.display())),
        _ => Err(format!("{}: drawn by several adapters", set.display())),
    }
}

/// What drawing one shot left: its manifest entry, and the adapter that drew it.
struct Drawn {
    shot: TakenShot,
    adapter: String,
}

/// Draws `shot` at `preset` through a `rift render` process of its own into the set under `out`,
/// by the adapter that draws it, which `adapter` names once known.
fn draw(
    game: &Path,
    scratch: &Path,
    out: &Path,
    name: &str,
    shot: &Shot,
    preset: VideoPreset,
    adapter: &str,
) -> Result<Drawn, String> {
    let mut shot = shot.clone();
    shot.video = Some(VideoChoice::Preset(preset));
    let file = scratch.join(format!("{name}-{}.json", preset.name()));
    write_json(&file, &shot)?;
    let arriving = shot.arrive.is_some();
    let relative = PathBuf::from(preset.name()).join(if arriving {
        name.to_owned()
    } else {
        format!("{name}.png")
    });
    let pending = if adapter.is_empty() {
        scratch.join("pending").join(&relative)
    } else {
        out.join(adapter).join(&relative)
    };
    if let Some(parent) = pending.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let rift = std::env::current_exe().map_err(|error| error.to_string())?;
    let drawn = Command::new(rift)
        .args(["render", "area"])
        .args([game, &file, &pending])
        .arg("--profile")
        .arg(scratch.join("profile"))
        .output()
        .map_err(|error| format!("rift doesn't run again: {error}"))?;
    let stdout = String::from_utf8_lossy(&drawn.stdout);
    let outcome: Option<RenderOutcome> = stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok());
    let (Some(outcome), true) = (outcome, drawn.status.success()) else {
        let stderr = String::from_utf8_lossy(&drawn.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(12).collect();
        eprintln!(
            "[snapshot] {name} at {} crashed ({}):\n{}",
            preset.name(),
            drawn.status,
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        );
        return Ok(Drawn {
            shot: TakenShot {
                name: name.to_owned(),
                video: preset,
                path: relative,
                width: shot.width,
                height: shot.height,
                hash: String::new(),
                frames: arriving.then_some(0),
                stood_after_frames: None,
                crashed: true,
                masks: Vec::new(),
            },
            adapter: adapter.to_owned(),
        });
    };
    let adapter = if adapter.is_empty() {
        let placed = out.join(&outcome.adapter).join(&relative);
        if let Some(parent) = placed.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        move_into(&pending, &placed)?;
        outcome.adapter.clone()
    } else {
        adapter.to_owned()
    };
    let placed = out.join(&adapter).join(&relative);
    let (hash, frames) = hash_of(&placed)?;
    Ok(Drawn {
        shot: TakenShot {
            name: name.to_owned(),
            video: preset,
            path: relative,
            width: shot.width,
            height: shot.height,
            hash,
            frames: arriving.then_some(frames),
            stood_after_frames: outcome.stood_after_frames,
            crashed: false,
            masks: Vec::new(),
        },
        adapter,
    })
}

/// Moves a picture or a folder of frames from one folder to another, across file systems too.
fn move_into(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        std::fs::create_dir_all(to).map_err(|error| error.to_string())?;
        for entry in std::fs::read_dir(from).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            std::fs::copy(entry.path(), to.join(entry.file_name()))
                .map_err(|error| error.to_string())?;
        }
        std::fs::remove_dir_all(from).map_err(|error| error.to_string())
    } else {
        std::fs::copy(from, to).map_err(|error| error.to_string())?;
        std::fs::remove_file(from).map_err(|error| error.to_string())
    }
}

/// The hash of a picture's bytes, or of every frame's in order, and how many frames there were.
pub fn hash_of(path: &Path) -> Result<(String, usize), String> {
    let mut hasher = blake3::Hasher::new();
    let frames = if path.is_dir() {
        let mut frames: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|frame| frame.extension().is_some_and(|ext| ext == "png"))
            .collect();
        frames.sort();
        frames
    } else {
        vec![path.to_path_buf()]
    };
    for frame in &frames {
        hasher.update(
            &std::fs::read(frame).map_err(|error| format!("{}: {error}", frame.display()))?,
        );
    }
    Ok((hasher.finalize().to_hex().to_string(), frames.len()))
}

/// Every shot of every shot file under `shots`, by name, with the presets it is drawn at.
fn expand_all(shots: &Path) -> Result<Vec<(String, Shot, Vec<VideoPreset>)>, String> {
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(shots)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".shot.json"))
        })
        .collect();
    files.sort();
    let mut expanded = Vec::new();
    for file in files {
        let shot_file: ShotFile = read_json(&file)?;
        let stem = file
            .file_name()
            .expect("a shot file's name")
            .to_string_lossy()
            .trim_end_matches(".shot.json")
            .to_owned();
        for (name, shot) in expand(&stem, &shot_file)? {
            expanded.push((name, shot, shot_file.presets.clone()));
        }
    }
    Ok(expanded)
}

/// The shots of a shot file: its shot over the cross product of its axes, each named after the
/// file and the axes' values.
fn expand(stem: &str, file: &ShotFile) -> Result<Vec<(String, Shot)>, String> {
    let matrix = &file.matrix;
    let mut shots = vec![(stem.to_owned(), file.shot.clone())];
    let frames_camera = !matrix.at.is_empty()
        || !matrix.pitch.is_empty()
        || !matrix.yaw.is_empty()
        || !matrix.zoom.is_empty();
    if frames_camera && !matches!(file.shot.view, ViewSpec::Camera(_)) {
        return Err(format!(
            "{stem}: a matrix of places, pitches, yaws or zooms frames the world's camera, so the shot's view must be `camera`"
        ));
    }
    shots = axis(shots, &matrix.at, |shot, at| {
        with_camera(shot, |camera| {
            camera.at = Spot {
                x: at.x,
                y: at.y,
                height: at.height,
            };
        });
        at.name.clone()
    });
    shots = axis(shots, &matrix.pitch, |shot, pitch| {
        with_camera(shot, |camera| camera.pitch = *pitch);
        format!("p{}", number(pitch.0))
    });
    shots = axis(shots, &matrix.yaw, |shot, yaw| {
        with_camera(shot, |camera| camera.yaw = *yaw);
        format!("y{}", number(yaw.0))
    });
    shots = axis(shots, &matrix.zoom, |shot, zoom| {
        with_camera(shot, |camera| camera.zoom = *zoom);
        format!("z{:?}", zoom).to_lowercase()
    });
    shots = axis(shots, &matrix.hour, |shot, hour| match hour {
        HourAxis::Fixed(_) => {
            shot.hour = None;
            "hfixed".to_owned()
        }
        HourAxis::At(hour) => {
            shot.hour = Some(*hour);
            format!("h{}", number(*hour))
        }
    });
    shots = axis(shots, &matrix.spell, |shot, spell| {
        shot.spell = Some(spell.clone());
        spell.to_lowercase().replace(' ', "-")
    });
    shots = axis(shots, &matrix.spell_phase, |shot, phase| {
        shot.spell_phase = *phase;
        format!("f{}", number(*phase))
    });
    Ok(shots)
}

/// Every shot so far crossed with every value of an axis, each named on by what `apply` says.
fn axis<T>(
    shots: Vec<(String, Shot)>,
    values: &[T],
    mut apply: impl FnMut(&mut Shot, &T) -> String,
) -> Vec<(String, Shot)> {
    if values.is_empty() {
        return shots;
    }
    shots
        .into_iter()
        .flat_map(|(name, shot)| {
            values
                .iter()
                .map(|value| {
                    let mut shot = shot.clone();
                    let tag = apply(&mut shot, value);
                    (format!("{name}-{tag}"), shot)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn with_camera(shot: &mut Shot, change: impl FnOnce(&mut CameraShot)) {
    if let ViewSpec::Camera(camera) = &mut shot.view {
        change(camera);
    }
}

/// A number as a name's part: whole when it is, else with its fraction, the point as a dash.
fn number(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}").replace('.', "-")
    }
}
