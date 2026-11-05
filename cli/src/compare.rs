//! How two sets of snapshots differ, shot by shot: a baseline set against a candidate set drawn by
//! another build. Each pair is measured three ways, so a change the picture as a whole hides
//! still shows: the structure of the whole picture's luminance, the structure of its worst
//! 64 × 64 tile (a prop or a patch of grass gone), and the share of its pixels whose colour shifted
//! more than the eye can tell. A shot passes within its allowance: its own, measured from the
//! noise of drawing it again and again, else the floor every shot gets.
//!
//! An arrival is a run of frames: its last, settled frame compares as a picture, and once the
//! area stood whole, no more of a frame's pixels may shift colour from the frame before than the
//! allowance lets a picture shift, else something popped in after the area stood. A fade passes,
//! a step at a time; a pop is a step too far.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use image::{GrayImage, RgbImage};
use image_compare::Algorithm;
use serde::{Deserialize, Serialize};

use crate::snapshot::{MANIFEST, Manifest, PictureRect, TakenShot};

/// What a run of the comparison ends with, as its exit code says.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Every shot within its allowance.
    Passed,
    /// A shot over its allowance.
    Regressed,
    /// The sets don't compare: a shot missing, sizes or adapters apart.
    Structural,
    /// A shot the candidate's render crashed on.
    Crashed,
}

impl Outcome {
    pub fn exit_code(self) -> i32 {
        match self {
            Outcome::Passed => 0,
            Outcome::Regressed => 1,
            Outcome::Structural => 2,
            Outcome::Crashed => 3,
        }
    }
}

/// How far a picture may stray from its baseline: how much structure the whole and its worst tile
/// may lose, and the share of pixels that may shift colour.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Allowance {
    pub mssim_loss: f64,
    pub tile_loss: f64,
    pub delta_e_share: f64,
}

impl Allowance {
    /// The least any shot is allowed: a drawing's own noise lies well under it.
    pub const FLOOR: Allowance = Allowance {
        mssim_loss: 0.002,
        tile_loss: 0.02,
        delta_e_share: 0.002,
    };

    /// What `metrics` break of the allowance, if anything.
    pub fn failure(&self, metrics: &Metrics) -> Option<String> {
        let mut broken = Vec::new();
        if 1.0 - metrics.mssim > self.mssim_loss {
            broken.push(format!(
                "structure {:.4} (allowed loss {:.4})",
                metrics.mssim, self.mssim_loss
            ));
        }
        if 1.0 - metrics.worst_tile > self.tile_loss {
            broken.push(format!(
                "worst tile {:.4} (allowed loss {:.4})",
                metrics.worst_tile, self.tile_loss
            ));
        }
        if metrics.delta_e_share > self.delta_e_share {
            broken.push(format!(
                "colour shift on {:.2}% of pixels (allowed {:.2}%)",
                metrics.delta_e_share * 100.0,
                self.delta_e_share * 100.0
            ));
        }
        (!broken.is_empty()).then(|| broken.join(", "))
    }

    /// An allowance `times` as wide as the loss `metrics` show, at least the floor.
    pub fn of_noise(metrics: &Metrics, times: f64) -> Allowance {
        Allowance {
            mssim_loss: ((1.0 - metrics.mssim) * times).max(Allowance::FLOOR.mssim_loss),
            tile_loss: ((1.0 - metrics.worst_tile) * times).max(Allowance::FLOOR.tile_loss),
            delta_e_share: (metrics.delta_e_share * times).max(Allowance::FLOOR.delta_e_share),
        }
    }
}

/// The allowances of a set of shots: each shot's own by its key, else the floor.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Thresholds {
    pub floor: Allowance,
    pub shots: BTreeMap<String, Allowance>,
}

impl Default for Thresholds {
    fn default() -> Thresholds {
        Thresholds {
            floor: Allowance::FLOOR,
            shots: BTreeMap::new(),
        }
    }
}

impl Thresholds {
    pub fn of(&self, key: &str) -> Allowance {
        self.shots.get(key).copied().unwrap_or(self.floor)
    }
}

/// How alike two pictures are: the structure of their luminance over the whole, the structure of
/// the tile that differs most, each 1 alike to 0 unlike, and the share of pixels whose colour
/// differs more than the eye can tell.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub mssim: f64,
    pub worst_tile: f64,
    pub delta_e_share: f64,
}

impl Metrics {
    /// The worse of each measure.
    pub fn worst(self, other: Metrics) -> Metrics {
        Metrics {
            mssim: self.mssim.min(other.mssim),
            worst_tile: self.worst_tile.min(other.worst_tile),
            delta_e_share: self.delta_e_share.max(other.delta_e_share),
        }
    }
}

/// What the comparison found, shot by shot, as `report.json` holds it.
#[derive(Serialize, Deserialize, Debug)]
pub struct Report {
    pub baseline_build: String,
    pub candidate_build: String,
    pub adapter: String,
    pub outcome: Outcome,
    pub shots: Vec<ShotReport>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ShotReport {
    pub key: String,
    pub allowance: Allowance,
    pub metrics: Option<Metrics>,
    /// Of an arrival, the largest share of pixels a frame after the area stood shifted in colour
    /// from the frame before it.
    pub popped_share: Option<f64>,
    pub stood_after_frames: Option<usize>,
    pub passed: bool,
    pub failure: Option<String>,
}

/// The side of the square tiles whose structure is measured one by one.
pub const TILE: u32 = 64;
/// The CIE76 colour difference past which a pixel counts as having shifted colour: a couple of
/// times the least difference the eye tells.
const DELTA_E_NOTICED: f64 = 5.0;
/// How much brighter than the difference itself the amplified difference is drawn.
const AMPLIFIED: u16 = 4;

/// Compares the candidate set against the baseline set, writing `report.json` and `report.html`
/// with a triptych per failing shot into `report` if given, taking the candidate's pictures over
/// the baseline's for the shots `accept` matches, within `thresholds` or the floor.
pub fn run(
    baseline: &Path,
    candidate: &Path,
    report: Option<&Path>,
    accept: Option<&str>,
    thresholds: Option<&Path>,
) -> Result<Outcome, String> {
    let before = Manifest::read(baseline)?;
    let after = Manifest::read(candidate)?;
    let thresholds = match thresholds {
        Some(path) => read_json(path)?,
        None => Thresholds::default(),
    };
    if before.adapter != after.adapter {
        eprintln!(
            "[compare] the sets were drawn by different adapters: {} against {}",
            before.adapter, after.adapter
        );
        return Ok(Outcome::Structural);
    }
    let mut shots = Vec::new();
    let mut outcome = Outcome::Passed;
    let worsen = |outcome: &mut Outcome, to: Outcome| {
        if to.exit_code() > outcome.exit_code() {
            *outcome = to;
        }
    };
    if let Some(report) = report {
        std::fs::create_dir_all(report).map_err(|error| error.to_string())?;
    }
    let mut triptychs = Vec::new();
    for taken in &after.shots {
        let key = taken.key();
        let allowance = thresholds.of(&key);
        let Some(was) = before.shots.iter().find(|was| was.key() == key) else {
            worsen(&mut outcome, Outcome::Structural);
            shots.push(failed(&key, allowance, taken, "not in the baseline"));
            continue;
        };
        if taken.crashed {
            worsen(&mut outcome, Outcome::Crashed);
            shots.push(failed(&key, allowance, taken, "the render crashed"));
            continue;
        }
        if was.crashed {
            worsen(&mut outcome, Outcome::Structural);
            shots.push(failed(
                &key,
                allowance,
                taken,
                "the baseline's render crashed",
            ));
            continue;
        }
        if (was.width, was.height) != (taken.width, taken.height) {
            worsen(&mut outcome, Outcome::Structural);
            shots.push(failed(&key, allowance, taken, "sizes differ"));
            continue;
        }
        let (a, b) = compared_pictures((baseline, was), (candidate, taken))?;
        let metrics = measure(&a, &b);
        let popped_share = match taken.stood_after_frames {
            Some(stood) if taken.frames.is_some() => Some(pop_in(candidate, taken, stood)?),
            _ => None,
        };
        let mut failure = allowance.failure(&metrics);
        if let Some(popped) = popped_share
            && popped > allowance.delta_e_share
        {
            let popped = format!(
                "after standing, {:.2}% of pixels shifted colour from one frame to the next (allowed {:.2}%)",
                popped * 100.0,
                allowance.delta_e_share * 100.0
            );
            failure = Some(match failure {
                Some(failure) => format!("{failure}; {popped}"),
                None => popped,
            });
        }
        if let Some(failure) = &failure {
            worsen(&mut outcome, Outcome::Regressed);
            if let Some(report) = report {
                triptychs.push(triptych(report, &key, &a, &b, &metrics, failure)?);
            }
        }
        shots.push(ShotReport {
            key,
            allowance,
            metrics: Some(metrics),
            popped_share,
            stood_after_frames: taken.stood_after_frames,
            passed: failure.is_none(),
            failure,
        });
    }
    for was in &before.shots {
        if !after.shots.iter().any(|taken| taken.key() == was.key()) {
            worsen(&mut outcome, Outcome::Structural);
            shots.push(failed(
                &was.key(),
                thresholds.of(&was.key()),
                was,
                "not in the candidate",
            ));
        }
    }
    let written = Report {
        baseline_build: before.build.clone(),
        candidate_build: after.build.clone(),
        adapter: after.adapter.clone(),
        outcome,
        shots,
    };
    for shot in &written.shots {
        println!(
            "[compare] {:<60} {}",
            shot.key,
            match (&shot.failure, shot.metrics) {
                (Some(failure), _) => format!("FAIL {failure}"),
                (None, Some(metrics)) => format!(
                    "ok   structure {:.4} worst tile {:.4} colour shift {:.2}%",
                    metrics.mssim,
                    metrics.worst_tile,
                    metrics.delta_e_share * 100.0
                ),
                (None, None) => "ok".to_owned(),
            }
        );
    }
    if let Some(report) = report {
        write_json(&report.join("report.json"), &written)?;
        std::fs::write(report.join("report.html"), html(&written, &triptychs))
            .map_err(|error| error.to_string())?;
        println!("[compare] report in {}", report.display());
    }
    if let Some(accept) = accept {
        accept_shots(baseline, candidate, &before, &after, &written, accept)?;
    }
    println!(
        "[compare] {} shots, {} failed: {:?}",
        written.shots.len(),
        written.shots.iter().filter(|shot| !shot.passed).count(),
        outcome
    );
    Ok(outcome)
}

/// How alike the two pictures are, by every measure.
pub fn measure(a: &RgbImage, b: &RgbImage) -> Metrics {
    let (luma_a, luma_b) = (image::imageops::grayscale(a), image::imageops::grayscale(b));
    Metrics {
        mssim: structure(&luma_a, &luma_b),
        worst_tile: worst_tile(&luma_a, &luma_b),
        delta_e_share: delta_e_share(a, b),
    }
}

fn structure(a: &GrayImage, b: &GrayImage) -> f64 {
    image_compare::gray_similarity_structure(&Algorithm::MSSIMSimple, a, b)
        .map_or(0.0, |similarity| similarity.score)
}

fn worst_tile(a: &GrayImage, b: &GrayImage) -> f64 {
    let (width, height) = a.dimensions();
    let mut worst = 1.0f64;
    for y in (0..height).step_by(TILE as usize) {
        for x in (0..width).step_by(TILE as usize) {
            let (w, h) = (TILE.min(width - x), TILE.min(height - y));
            let tile_a = image::imageops::crop_imm(a, x, y, w, h).to_image();
            let tile_b = image::imageops::crop_imm(b, x, y, w, h).to_image();
            worst = worst.min(structure(&tile_a, &tile_b));
        }
    }
    worst
}

fn delta_e_share(a: &RgbImage, b: &RgbImage) -> f64 {
    let pixels = a.pixels().zip(b.pixels());
    let total = a.pixels().len().max(1);
    let shifted = pixels
        .filter(|(a, b)| {
            let (a, b) = (lab(a.0), lab(b.0));
            (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt() > DELTA_E_NOTICED
        })
        .count();
    shifted as f64 / total as f64
}

/// CIE L*a*b* of an sRGB pixel, against the D65 white.
fn lab([r, g, b]: [u8; 3]) -> [f64; 3] {
    let linear = |channel: u8| {
        let c = f64::from(channel) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (linear(r), linear(g), linear(b));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f64| {
        if t > 0.008856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// The settled pictures of a shot in two sets, each in its set's folder, with what either masks
/// painted over in both.
pub fn compared_pictures(
    (set_a, a): (&Path, &TakenShot),
    (set_b, b): (&Path, &TakenShot),
) -> Result<(RgbImage, RgbImage), String> {
    let (mut picture_a, mut picture_b) = (settled_picture(set_a, a)?, settled_picture(set_b, b)?);
    for mask in a.masks.iter().chain(&b.masks) {
        blank(&mut picture_a, mask);
        blank(&mut picture_b, mask);
    }
    Ok((picture_a, picture_b))
}

/// The picture of `taken` as it settled: the picture itself, or the last frame of an arrival.
fn settled_picture(set: &Path, taken: &TakenShot) -> Result<RgbImage, String> {
    let path = set.join(&taken.path);
    let picture = match taken.frames {
        Some(_) => frame_paths(&path)?
            .pop()
            .ok_or_else(|| format!("{}: no frames", path.display()))?,
        None => path,
    };
    load(&picture)
}

fn frame_paths(folder: &Path) -> Result<Vec<PathBuf>, String> {
    let mut frames: Vec<PathBuf> = std::fs::read_dir(folder)
        .map_err(|error| format!("{}: {error}", folder.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .collect();
    frames.sort();
    Ok(frames)
}

/// The largest share of pixels any frame of the arrival `taken` shifts in colour from the frame
/// before it, from the frame after the one the area stood on, which starts the picture's
/// smoothing over.
fn pop_in(set: &Path, taken: &TakenShot, stood: usize) -> Result<f64, String> {
    let frames = frame_paths(&set.join(&taken.path))?;
    let mut worst: f64 = 0.0;
    let mut before: Option<RgbImage> = None;
    for frame in frames.iter().skip(stood + 1) {
        let frame = load(frame)?;
        if let Some(before) = &before {
            worst = worst.max(delta_e_share(before, &frame));
        }
        before = Some(frame);
    }
    Ok(worst)
}

/// Paints `mask` of `picture` over in one colour, so whatever moved there compares alike.
fn blank(picture: &mut RgbImage, mask: &PictureRect) {
    let (width, height) = picture.dimensions();
    for y in mask.y.min(height)..(mask.y + mask.height).min(height) {
        for x in mask.x.min(width)..(mask.x + mask.width).min(width) {
            picture.put_pixel(x, y, image::Rgb(MASKED));
        }
    }
}

/// The colour masked pixels are painted over in.
const MASKED: [u8; 3] = [255, 0, 255];

fn load(path: &Path) -> Result<RgbImage, String> {
    image::open(path)
        .map(|picture| picture.to_rgb8())
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn failed(key: &str, allowance: Allowance, taken: &TakenShot, why: &str) -> ShotReport {
    ShotReport {
        key: key.to_owned(),
        allowance,
        metrics: None,
        popped_share: None,
        stood_after_frames: taken.stood_after_frames,
        passed: false,
        failure: Some(why.to_owned()),
    }
}

/// A failing shot's three pictures written beside the report: the baseline, the candidate and
/// their difference amplified; and the words the page shows with them.
struct Triptych {
    key: String,
    baseline: PathBuf,
    candidate: PathBuf,
    difference: PathBuf,
    caption: String,
}

fn triptych(
    report: &Path,
    key: &str,
    a: &RgbImage,
    b: &RgbImage,
    metrics: &Metrics,
    failure: &str,
) -> Result<Triptych, String> {
    let stem = key.replace('/', "-");
    let folder = report.join("triptychs");
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let mut difference = RgbImage::new(a.width(), a.height());
    for ((a, b), out) in a.pixels().zip(b.pixels()).zip(difference.pixels_mut()) {
        for i in 0..3 {
            let apart = u16::from(a.0[i].abs_diff(b.0[i]));
            out.0[i] = (apart * AMPLIFIED).min(255) as u8;
        }
    }
    let paths = [
        folder.join(format!("{stem}.baseline.png")),
        folder.join(format!("{stem}.candidate.png")),
        folder.join(format!("{stem}.difference.png")),
    ];
    a.save(&paths[0]).map_err(|error| error.to_string())?;
    b.save(&paths[1]).map_err(|error| error.to_string())?;
    difference
        .save(&paths[2])
        .map_err(|error| error.to_string())?;
    let [baseline, candidate, difference] = paths;
    Ok(Triptych {
        key: key.to_owned(),
        baseline: baseline
            .strip_prefix(report)
            .expect("under the report")
            .to_path_buf(),
        candidate: candidate
            .strip_prefix(report)
            .expect("under the report")
            .to_path_buf(),
        difference: difference
            .strip_prefix(report)
            .expect("under the report")
            .to_path_buf(),
        caption: format!(
            "{failure}. Structure {:.4}, worst tile {:.4}, colour shift on {:.2}% of pixels.",
            metrics.mssim,
            metrics.worst_tile,
            metrics.delta_e_share * 100.0
        ),
    })
}

fn html(report: &Report, triptychs: &[Triptych]) -> String {
    let mut page = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Snapshot comparison</title>\
         <style>body{{font-family:sans-serif;background:#111;color:#ddd;margin:1em}}\
         figure{{margin:1em 0}}figure img{{width:32%;image-rendering:auto}}\
         figcaption{{margin-top:.3em}}.fail{{color:#f66}}.ok{{color:#6c6}}\
         table{{border-collapse:collapse}}td,th{{padding:.2em .6em;text-align:left}}</style></head><body>\
         <h1>Snapshots: {} against {}</h1><p>Adapter {}. Outcome <b>{:?}</b>.</p>",
        report.candidate_build, report.baseline_build, report.adapter, report.outcome
    );
    for triptych in triptychs {
        page.push_str(&format!(
            "<figure><h2 class=\"fail\">{}</h2>\
             <img src=\"{}\" alt=\"baseline\"> <img src=\"{}\" alt=\"candidate\"> <img src=\"{}\" alt=\"difference\">\
             <figcaption>{}</figcaption></figure>",
            triptych.key,
            triptych.baseline.display(),
            triptych.candidate.display(),
            triptych.difference.display(),
            triptych.caption
        ));
    }
    page.push_str("<table><tr><th>Shot</th><th>Structure</th><th>Worst tile</th><th>Colour shift</th><th>Stood after</th><th></th></tr>");
    for shot in &report.shots {
        let (mssim, tile, shift) = shot.metrics.map_or(
            ("-".to_owned(), "-".to_owned(), "-".to_owned()),
            |metrics| {
                (
                    format!("{:.4}", metrics.mssim),
                    format!("{:.4}", metrics.worst_tile),
                    format!("{:.2}%", metrics.delta_e_share * 100.0),
                )
            },
        );
        page.push_str(&format!(
            "<tr><td>{}</td><td>{mssim}</td><td>{tile}</td><td>{shift}</td><td>{}</td><td class=\"{}\">{}</td></tr>",
            shot.key,
            shot.stood_after_frames
                .map_or(String::new(), |frames| format!("{frames} frames")),
            if shot.passed { "ok" } else { "fail" },
            shot.failure.as_deref().unwrap_or("ok")
        ));
    }
    page.push_str("</table></body></html>");
    page
}

/// Takes the candidate's pictures over the baseline's for every shot `glob` matches, and notes
/// each in the baseline's `ACCEPTED.md`.
fn accept_shots(
    baseline: &Path,
    candidate: &Path,
    before: &Manifest,
    after: &Manifest,
    report: &Report,
    glob: &str,
) -> Result<(), String> {
    let mut accepted = before.clone();
    let mut notes = String::new();
    for taken in &after.shots {
        let key = taken.key();
        if !matches(glob, &key) || taken.crashed {
            continue;
        }
        let from = candidate.join(&taken.path);
        let to = baseline.join(&taken.path);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if from.is_dir() {
            let _ = std::fs::remove_dir_all(&to);
            std::fs::create_dir_all(&to).map_err(|error| error.to_string())?;
            for frame in frame_paths(&from)? {
                let name = frame.file_name().expect("a frame's name");
                std::fs::copy(&frame, to.join(name)).map_err(|error| error.to_string())?;
            }
        } else {
            std::fs::copy(&from, &to).map_err(|error| error.to_string())?;
        }
        match accepted.shots.iter_mut().find(|was| was.key() == key) {
            Some(was) => *was = taken.clone(),
            None => accepted.shots.push(taken.clone()),
        }
        let shot = report.shots.iter().find(|shot| shot.key == key);
        let metrics =
            shot.and_then(|shot| shot.metrics)
                .map_or("no measure".to_owned(), |metrics| {
                    format!(
                        "structure {:.4}, worst tile {:.4}, colour shift {:.2}%",
                        metrics.mssim,
                        metrics.worst_tile,
                        metrics.delta_e_share * 100.0
                    )
                });
        notes.push_str(&format!(
            "- {} {key}: {} → {} ({metrics})\n",
            chrono::Local::now().format("%Y-%m-%d"),
            before.build,
            after.build
        ));
        println!("[compare] accepted {key}");
    }
    accepted.build = after.build.clone();
    write_json(&baseline.join(MANIFEST), &accepted)?;
    let log = baseline.join("ACCEPTED.md");
    let mut written = std::fs::read_to_string(&log).unwrap_or_default();
    written.push_str(&notes);
    std::fs::write(&log, written).map_err(|error| error.to_string())
}

/// Whether `text` matches `pattern`, where `*` stands for any run of characters and `?` for one.
pub fn matches(pattern: &str, text: &str) -> bool {
    fn go(pattern: &[u8], text: &[u8]) -> bool {
        match (pattern.first(), text.first()) {
            (None, None) => true,
            (None, Some(_)) => false,
            (Some(b'*'), _) => {
                go(&pattern[1..], text) || (!text.is_empty() && go(pattern, &text[1..]))
            }
            (Some(b'?'), Some(_)) => go(&pattern[1..], &text[1..]),
            (Some(p), Some(t)) => p == t && go(&pattern[1..], &text[1..]),
            (Some(_), None) => false,
        }
    }
    go(pattern.as_bytes(), text.as_bytes())
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, json).map_err(|error| format!("{}: {error}", path.display()))
}
