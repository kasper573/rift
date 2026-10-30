use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::sync::{LazyLock, Mutex};

use super::gltf::{self, GltfImage, GltfInfo};
use super::load::{FileHash, FileHashes, Loading};
use super::report::{ContentProblem, ErrorPlace};
use crate::core::assets::AssetKind;
use crate::core::audio;

pub struct ImageSizes(pub HashMap<String, (u32, u32)>);

/// What the loader learnt of every model, by its path from the game folder.
pub struct GltfInfos(pub HashMap<String, GltfInfo>);

/// What decoding a media file found: an image's size, a model's measure, nothing more for a sound
/// or a font, or why it does not decode.
type MediaCheck = Result<Decoded, MediaProblem>;

#[derive(Clone)]
enum Decoded {
    Plain,
    Image((u32, u32)),
    Model(GltfInfo),
}

#[derive(Clone)]
struct MediaProblem {
    rule: &'static str,
    found: String,
    expected: &'static str,
}

/// Every media file is decoded whole, as the game decodes it, so a file damaged anywhere fails the
/// load rather than showing nothing in play.
pub async fn check_media(loading: &mut Loading) {
    let mut paths: Vec<String> = loading.media().to_vec();
    paths.extend(loading.media_parts());
    let folder = loading.folder();
    let hashes = loading.content().built::<FileHashes>().0.clone();
    let checked: Vec<(String, MediaCheck)> = loading
        .each(paths, |path| {
            let checked_already = folder.checked(&path).is_some();
            let read = || match folder.read(&path) {
                Ok(bytes) if checked_already => measure(&path, &bytes),
                Ok(bytes) => decode(&path, &bytes),
                Err(error) => Err(MediaProblem {
                    rule: "unreadable file",
                    found: error.to_string(),
                    expected: "a readable file",
                }),
            };
            let check = match hashes.get(&path) {
                Some(hashed) => remembered(&hashed.hash, read),
                None => read(),
            };
            (path, check)
        })
        .await;
    let mut sizes = HashMap::new();
    let mut models = HashMap::new();
    for (path, result) in checked {
        match result {
            Ok(Decoded::Image(size)) => {
                sizes.insert(path, size);
            }
            Ok(Decoded::Model(info)) => {
                for problem in image_problems(&path, &info, &hashes) {
                    loading.file_error(path.clone(), ErrorPlace::WholeFile, problem);
                }
                models.insert(path, info);
            }
            Ok(Decoded::Plain) => {}
            Err(problem) => loading.file_error(
                path,
                ErrorPlace::WholeFile,
                ContentProblem::new(problem.rule, problem.found, problem.expected),
            ),
        }
    }
    loading.built(ImageSizes(sizes));
    loading.built(GltfInfos(models));
}

/// What a model's images must be: PNG files beside it that the game holds, never kept inside it,
/// so each is read at its own pace and only when the model is drawn.
fn image_problems(
    model: &str,
    info: &GltfInfo,
    held: &BTreeMap<String, FileHash>,
) -> Vec<ContentProblem> {
    let folder = model.rsplit_once('/').map_or("", |(folder, _)| folder);
    let mut problems = Vec::new();
    let embedded = info
        .images
        .iter()
        .filter(|image| **image == GltfImage::Embedded)
        .count();
    if embedded > 0 {
        problems.push(ContentProblem::new(
            "embedded image",
            format!(
                "{embedded} {} kept inside the model",
                if embedded == 1 { "image" } else { "images" }
            ),
            "the model's images as PNG files beside it, each named by its path from the model's folder",
        ));
    }
    for image in &info.images {
        let GltfImage::File(uri) = image else {
            continue;
        };
        let path = if folder.is_empty() {
            uri.clone()
        } else {
            format!("{folder}/{uri}")
        };
        let held = !uri.contains("..")
            && AssetKind::of(uri) == Some(AssetKind::Image)
            && held.contains_key(&path);
        if !held {
            problems.push(ContentProblem::new(
                "missing image",
                format!("the model names {uri}"),
                "a PNG file beside the model that the game holds, as a part of its bundle",
            ));
        }
    }
    problems
}

/// Decoding the same bytes again finds the same, so each is decoded once per process however often
/// a game is loaded, as the editor does on every edit.
fn remembered(hash: &str, decode: impl FnOnce() -> MediaCheck) -> MediaCheck {
    static DECODED: LazyLock<Mutex<HashMap<String, MediaCheck>>> = LazyLock::new(Mutex::default);
    if let Some(known) = DECODED
        .lock()
        .ok()
        .and_then(|known| known.get(hash).cloned())
    {
        return known;
    }
    let check = decode();
    if let Ok(mut known) = DECODED.lock() {
        known.insert(hash.to_owned(), check.clone());
    }
    check
}

/// The size in pixels of the image at `path` of `bytes`, hashed `hash` when known, when it
/// decodes.
pub(super) fn image_size(path: &str, hash: Option<&str>, bytes: &[u8]) -> Option<(u32, u32)> {
    match decoded(path, hash, bytes)? {
        Decoded::Image(size) => Some(size),
        _ => None,
    }
}

/// What the model at `path` of `bytes`, hashed `hash` when known, holds, when it reads.
pub fn gltf_info(path: &str, hash: Option<&str>, bytes: &[u8]) -> Option<GltfInfo> {
    match decoded(path, hash, bytes)? {
        Decoded::Model(info) => Some(info),
        _ => None,
    }
}

fn decoded(path: &str, hash: Option<&str>, bytes: &[u8]) -> Option<Decoded> {
    let hash = hash.map_or_else(|| FileHash::of(bytes).hash, str::to_owned);
    remembered(&hash, || decode(path, bytes)).ok()
}

/// Why the media file at `path` of `bytes` does not decode, when it does not.
pub(super) fn problem_of(path: &str, bytes: &[u8]) -> Option<ContentProblem> {
    remembered(&FileHash::of(bytes).hash, || decode(path, bytes))
        .err()
        .map(|problem| ContentProblem::new(problem.rule, problem.found, problem.expected))
}

fn decode(path: &str, bytes: &[u8]) -> MediaCheck {
    match AssetKind::of(path) {
        Some(AssetKind::Image) => decode_png(bytes)
            .map(Decoded::Image)
            .map_err(unreadable_image),
        Some(AssetKind::Audio) => audio::loader::decode(bytes.to_vec())
            .and_then(audio::loader::DecodedAudio::into_sound)
            .map(|_| Decoded::Plain)
            .map_err(|error| MediaProblem {
                rule: "unreadable sound",
                found: error.to_string(),
                expected: "a WAV or FLAC file of one or two channels that decodes",
            }),
        Some(AssetKind::Model) => {
            gltf::inspect(bytes)
                .map(Decoded::Model)
                .map_err(|found| MediaProblem {
                    rule: "unreadable model",
                    found,
                    expected: "a glTF binary that validates",
                })
        }
        None => Ok(Decoded::Plain),
    }
}

/// What a media file this build's loader already checked finds, read only as far as the game
/// needs of it: an image's size from its header, a model's measure, nothing of a sound or a font.
fn measure(path: &str, bytes: &[u8]) -> MediaCheck {
    match AssetKind::of(path) {
        Some(AssetKind::Image) => png_size(bytes)
            .map(Decoded::Image)
            .map_err(unreadable_image),
        Some(AssetKind::Model) => decode(path, bytes),
        Some(AssetKind::Audio) | None => Ok(Decoded::Plain),
    }
}

fn unreadable_image(found: String) -> MediaProblem {
    MediaProblem {
        rule: "unreadable image",
        found,
        expected: "a PNG that decodes",
    }
}

pub struct VisiblePixelColumns {
    pub first: u32,
    pub last: u32,
    pub image_width: u32,
}

pub fn visible_columns(bytes: &[u8]) -> Result<Option<VisiblePixelColumns>, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(
        png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| "an image too large to decode".to_owned())?;
    let mut pixels = vec![0; size];
    let frame = reader
        .next_frame(&mut pixels)
        .map_err(|error| error.to_string())?;
    let channels = frame.color_type.samples();
    let alpha = channels - 1;
    let mut covered: Option<(u32, u32)> = None;
    for row in pixels[..frame.buffer_size()].chunks_exact(frame.line_size) {
        let opaque = |(column, pixel): (usize, &[u8])| (pixel[alpha] > 0).then_some(column as u32);
        let mut columns = row[..frame.width as usize * channels]
            .chunks_exact(channels)
            .enumerate()
            .filter_map(opaque);
        let Some(first) = columns.next() else {
            continue;
        };
        let last = columns.next_back().unwrap_or(first);
        covered = Some(covered.map_or((first, last), |(left, right)| {
            (left.min(first), right.max(last))
        }));
    }
    Ok(covered.map(|(first, last)| VisiblePixelColumns {
        first,
        last,
        image_width: frame.width,
    }))
}

fn png_size(bytes: &[u8]) -> Result<(u32, u32), String> {
    let reader = png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .map_err(|error| error.to_string())?;
    let info = reader.info();
    Ok((info.width, info.height))
}

fn decode_png(bytes: &[u8]) -> Result<(u32, u32), String> {
    let mut reader = png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .map_err(|error| error.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| "an image too large to decode".to_owned())?;
    let mut pixels = vec![0; size];
    let frame = reader
        .next_frame(&mut pixels)
        .map_err(|error| error.to_string())?;
    Ok((frame.width, frame.height))
}
