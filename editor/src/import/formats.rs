use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::{AnimationDecoder, GenericImage, ImageDecoder, RgbaImage};
use serde::Deserialize;

const MAX_FRAME_PX: u32 = 8192;

pub const MAX_IMAGE_PX: u32 = 16384;
pub const DEFAULT_MS: u32 = 100;

#[derive(Clone, Debug)]
pub struct AnimationExport {
    pub files: Vec<PathBuf>,
    pub kind: ExportFormat,
    pub frame: (u32, u32),
    pub trimmed: bool,
    pub strips: Vec<ExportStrip>,
    pub warnings: Vec<String>,
    pub sheet: Option<RgbaImage>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    SheetJson,
    Grid,
    Sequence,
    Gif,
    Apng,
}

#[derive(Clone, Debug)]
pub struct ExportStrip {
    pub tag: String,
    pub frames: Vec<ExportFrame>,
}

#[derive(Clone, Debug)]
pub struct ExportFrame {
    pub image: RgbaImage,
    pub ms: Option<u32>,
    pub hash: String,
}

impl ExportFormat {
    pub fn sentence(self) -> &'static str {
        match self {
            ExportFormat::SheetJson => "a sheet and its JSON",
            ExportFormat::Grid => "a sheet on a grid",
            ExportFormat::Sequence => "an image sequence",
            ExportFormat::Gif => "an animated GIF",
            ExportFormat::Apng => "an APNG",
        }
    }
}

impl AnimationExport {
    pub fn new(
        files: Vec<PathBuf>,
        kind: ExportFormat,
        strips: Vec<ExportStrip>,
    ) -> AnimationExport {
        AnimationExport {
            files,
            kind,
            frame: (0, 0),
            trimmed: false,
            strips,
            warnings: Vec::new(),
            sheet: None,
        }
    }

    pub fn frames(&self) -> usize {
        self.strips.iter().map(|strip| strip.frames.len()).sum()
    }

    pub fn timed(&self) -> bool {
        matches!(
            self.kind,
            ExportFormat::SheetJson | ExportFormat::Gif | ExportFormat::Apng
        ) && self
            .strips
            .iter()
            .all(|strip| strip.frames.iter().all(|frame| frame.ms.is_some()))
    }

    pub fn strip(&self, tag: &str) -> Option<&ExportStrip> {
        self.strips.iter().find(|strip| strip.tag == tag)
    }

    pub fn cut_grid(&mut self, width: u32, height: u32) -> Result<(), String> {
        let sheet = self.sheet.as_ref().ok_or("not a grid")?;
        if width == 0 || height == 0 {
            return Err("a frame size is at least 1 × 1".to_owned());
        }
        if width > MAX_FRAME_PX || height > MAX_FRAME_PX {
            return Err(format!(
                "frames are at most {MAX_FRAME_PX} × {MAX_FRAME_PX}"
            ));
        }
        if sheet.width() % width != 0 || sheet.height() % height != 0 {
            return Err(format!(
                "{} × {} does not divide into {width} × {height} frames",
                sheet.width(),
                sheet.height()
            ));
        }
        let columns = sheet.width() / width;
        let rows = sheet.height() / height;
        self.frame = (width, height);
        self.strips = (0..rows)
            .map(|row| ExportStrip {
                tag: format!("row {}", row + 1),
                frames: (0..columns)
                    .map(|column| {
                        let image = image::imageops::crop_imm(
                            sheet,
                            column * width,
                            row * height,
                            width,
                            height,
                        )
                        .to_image();
                        ExportFrame::untimed(image)
                    })
                    .collect(),
            })
            .collect();
        Ok(())
    }
}

impl ExportFrame {
    pub fn new(image: RgbaImage, ms: Option<u32>) -> ExportFrame {
        let hash = pixel_hash(&image);
        ExportFrame { image, ms, hash }
    }

    pub fn untimed(image: RgbaImage) -> ExportFrame {
        ExportFrame::new(image, None)
    }
}

pub fn pixel_hash(image: &RgbaImage) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&image.width().to_le_bytes());
    hasher.update(&image.height().to_le_bytes());
    for pixel in image.pixels() {
        match pixel[3] {
            0 => hasher.update(&[0, 0, 0, 0]),
            _ => hasher.update(&pixel.0),
        };
    }
    hasher.finalize().to_hex()[..16].to_owned()
}

pub fn is_blank(image: &RgbaImage) -> bool {
    image.pixels().all(|pixel| pixel[3] == 0)
}

pub fn cell(sheet: &RgbaImage, size: (u32, u32), index: u32) -> Option<RgbaImage> {
    let (width, height) = size;
    let columns = sheet
        .width()
        .checked_div(width)
        .filter(|columns| *columns > 0)?;
    let (x, y) = ((index % columns) * width, (index / columns) * height);
    (y + height <= sheet.height())
        .then(|| image::imageops::crop_imm(sheet, x, y, width, height).to_image())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DroppedFiles {
    Sheet {
        image: Option<PathBuf>,
        json: PathBuf,
    },
    Png(PathBuf),
    Apng(PathBuf),
    Gif(PathBuf),
    Sequence(Vec<PathBuf>),
}

impl DroppedFiles {
    pub fn files(&self) -> Vec<PathBuf> {
        match self {
            DroppedFiles::Sheet { image, json } => image.iter().chain([json]).cloned().collect(),
            DroppedFiles::Png(file) | DroppedFiles::Apng(file) | DroppedFiles::Gif(file) => {
                vec![file.clone()]
            }
            DroppedFiles::Sequence(files) => files.clone(),
        }
    }
}

pub const FORMATS: &str = "a sheet with its JSON (Aseprite or TexturePacker), a sheet on a grid, a numbered PNG sequence, an animated GIF or APNG; a tileset is a grid; a bust is one PNG";

pub fn sort_out(files: &[PathBuf]) -> Result<DroppedFiles, String> {
    if files.is_empty() {
        return Err(format!("nothing to import; the editor imports {FORMATS}"));
    }
    let mut pngs = Vec::new();
    let mut jsons = Vec::new();
    let mut gifs = Vec::new();
    let mut others = Vec::new();
    for file in files {
        match header_of(file) {
            Header::Png => pngs.push(file.clone()),
            Header::Gif => gifs.push(file.clone()),
            Header::Json => jsons.push(file.clone()),
            Header::Other => others.push(file.clone()),
        }
    }
    if !others.is_empty() {
        return Err(format!(
            "{} is not an export; the editor imports {FORMATS}",
            names(&others)
        ));
    }
    match (jsons.len(), pngs.len(), gifs.len()) {
        (1, 0 | 1, 0) => Ok(DroppedFiles::Sheet {
            image: pngs.pop(),
            json: jsons.remove(0),
        }),
        (0, 0, 1) => Ok(DroppedFiles::Gif(gifs.remove(0))),
        (0, 1, 0) => {
            let png = pngs.remove(0);
            if is_apng(&png) {
                Ok(DroppedFiles::Apng(png))
            } else {
                Ok(DroppedFiles::Png(png))
            }
        }
        (0, n, 0) if n > 1 => {
            pngs.sort();
            Ok(DroppedFiles::Sequence(pngs))
        }
        _ => Err(format!(
            "{} mix formats; the editor imports {FORMATS}, one export at a time",
            names(files)
        )),
    }
}

pub fn decode(dropped: &DroppedFiles) -> Result<AnimationExport, String> {
    match dropped {
        DroppedFiles::Sheet { image, json } => decode_sheet(image.as_deref(), json),
        DroppedFiles::Png(file) => decode_grid(file),
        DroppedFiles::Apng(file) => decode_apng(file),
        DroppedFiles::Gif(file) => decode_gif(file),
        DroppedFiles::Sequence(files) => decode_sequence(files),
    }
}

pub fn decode_image(file: &Path) -> Result<RgbaImage, String> {
    let bytes = std::fs::read(file).map_err(|error| format!("{}: {error}", name(file)))?;
    decode_image_bytes(&bytes).map_err(|error| format!("{}: {error}", name(file)))
}

pub fn decode_image_bytes(bytes: &[u8]) -> Result<RgbaImage, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    reader.limits(limits());
    reader
        .decode()
        .map(|image| image.into_rgba8())
        .map_err(|error| error.to_string())
}

pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

pub fn name(file: &Path) -> String {
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string())
}

pub fn names(files: &[PathBuf]) -> String {
    files
        .iter()
        .map(|file| name(file))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn export_name(files: &[PathBuf]) -> String {
    match files {
        [image, json] if json.extension().is_some_and(|ext| ext == "json") => {
            format!("{} and .json", name(image))
        }
        [one] => name(one),
        many => format!("{} files", many.len()),
    }
}

pub fn stem(file: &Path) -> String {
    file.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_PX);
    limits.max_image_height = Some(MAX_IMAGE_PX);
    limits
}

enum Header {
    Png,
    Gif,
    Json,
    Other,
}

fn header_of(file: &Path) -> Header {
    let Ok(bytes) = std::fs::read(file) else {
        return Header::Other;
    };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Header::Png;
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Header::Gif;
    }
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(64)]);
    if text.trim_start().starts_with('{') {
        return Header::Json;
    }
    Header::Other
}

fn is_apng(file: &Path) -> bool {
    File::open(file)
        .ok()
        .and_then(|file| PngDecoder::new(BufReader::new(file)).ok())
        .and_then(|decoder| decoder.is_apng().ok())
        .unwrap_or(false)
}

#[derive(Deserialize)]
struct SheetJson {
    frames: FramesJson,
    #[serde(default)]
    meta: MetaJson,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FramesJson {
    Map(BTreeMap<String, FrameJson>),
    List(Vec<FrameJson>),
}

#[derive(Deserialize)]
struct FrameJson {
    #[serde(default)]
    filename: String,
    frame: RectJson,
    #[serde(default)]
    rotated: bool,
    #[serde(default)]
    trimmed: bool,
    #[serde(rename = "spriteSourceSize")]
    sprite_source_size: Option<RectJson>,
    #[serde(rename = "sourceSize")]
    source_size: Option<SizeJson>,
    duration: Option<u32>,
}

#[derive(Deserialize, Clone, Copy)]
struct RectJson {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Deserialize, Clone, Copy)]
struct SizeJson {
    w: u32,
    h: u32,
}

#[derive(Deserialize, Default)]
struct MetaJson {
    image: Option<String>,
    #[serde(rename = "frameTags", default)]
    frame_tags: Vec<TagJson>,
}

#[derive(Deserialize)]
struct TagJson {
    name: String,
    from: usize,
    to: usize,
}

fn decode_sheet(image_file: Option<&Path>, json_file: &Path) -> Result<AnimationExport, String> {
    let text = std::fs::read_to_string(json_file)
        .map_err(|error| format!("{}: {error}", name(json_file)))?;
    let sheet: SheetJson =
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", name(json_file)))?;
    let image_file = match image_file {
        Some(image_file) => image_file.to_path_buf(),
        None => image_beside(json_file, &sheet.meta)?,
    };
    let image = decode_image(&image_file)?;
    let frames: Vec<FrameJson> = match sheet.frames {
        FramesJson::List(list) => list,
        FramesJson::Map(map) => {
            let mut frames: Vec<FrameJson> = map
                .into_iter()
                .map(|(filename, frame)| FrameJson { filename, ..frame })
                .collect();
            frames.sort_by_cached_key(|frame| natural(&stem(Path::new(&frame.filename))));
            frames
        }
    };
    if frames.is_empty() {
        return Err(format!("{} lists no frames", name(json_file)));
    }
    let mut trimmed = false;
    let mut decoded = Vec::new();
    for frame in &frames {
        trimmed |= frame.trimmed || frame.rotated;
        let (region_w, region_h) = if frame.rotated {
            (frame.frame.h, frame.frame.w)
        } else {
            (frame.frame.w, frame.frame.h)
        };
        if frame.frame.x + region_w > image.width() || frame.frame.y + region_h > image.height() {
            return Err(format!(
                "{} lies outside {}",
                frame.filename,
                name(&image_file)
            ));
        }
        let mut sprite =
            image::imageops::crop_imm(&image, frame.frame.x, frame.frame.y, region_w, region_h)
                .to_image();
        if frame.rotated {
            sprite = image::imageops::rotate270(&sprite);
        }
        let source = frame.source_size.unwrap_or(SizeJson {
            w: sprite.width(),
            h: sprite.height(),
        });
        let offset = frame
            .sprite_source_size
            .map(|r| (r.x, r.y))
            .unwrap_or((0, 0));
        let mut canvas = RgbaImage::new(source.w.max(1), source.h.max(1));
        if offset.0 + sprite.width() > canvas.width()
            || offset.1 + sprite.height() > canvas.height()
        {
            return Err(format!("{} does not fit its source size", frame.filename));
        }
        canvas
            .copy_from(&sprite, offset.0, offset.1)
            .map_err(|error| error.to_string())?;
        decoded.push(ExportFrame::new(canvas, frame.duration));
    }
    let tags: Vec<(String, Vec<usize>)> = if sheet.meta.frame_tags.is_empty() {
        by_tag(frames.iter().enumerate().map(|(index, frame)| {
            let (tag, number) = split_number(&stem(Path::new(&frame.filename)));
            (tag, number.unwrap_or(index as u64), index)
        }))
        .into_iter()
        .map(|(tag, members)| (tag, members.into_iter().map(|(_, index)| index).collect()))
        .collect()
    } else {
        let mut seen = vec![None; frames.len()];
        let mut tags = Vec::new();
        for tag in &sheet.meta.frame_tags {
            if tag.name.trim().is_empty() {
                return Err(format!("{} has an unnamed tag", name(json_file)));
            }
            if tag.to < tag.from || tag.to >= frames.len() {
                return Err(format!("tag {} is empty", tag.name));
            }
            for (index, slot) in seen.iter_mut().enumerate().take(tag.to + 1).skip(tag.from) {
                if let Some(other) = slot {
                    return Err(format!(
                        "tags {} and {other} share frame {index}; a frame belongs to one strip",
                        tag.name
                    ));
                }
                *slot = Some(tag.name.clone());
            }
            tags.push((tag.name.clone(), (tag.from..=tag.to).collect()));
        }
        tags
    };
    let strips = tags
        .into_iter()
        .map(|(tag, indices)| ExportStrip {
            tag,
            frames: indices
                .into_iter()
                .map(|index| decoded[index].clone())
                .collect(),
        })
        .collect();
    let mut export = AnimationExport::new(
        vec![image_file, json_file.to_path_buf()],
        ExportFormat::SheetJson,
        strips,
    );
    export.trimmed = trimmed;
    unify(&mut export)?;
    Ok(export)
}

fn decode_grid(file: &Path) -> Result<AnimationExport, String> {
    let image = decode_image(file)?;
    let mut export = AnimationExport::new(vec![file.to_path_buf()], ExportFormat::Grid, Vec::new());
    export.sheet = Some(image);
    Ok(export)
}

fn decode_sequence(files: &[PathBuf]) -> Result<AnimationExport, String> {
    let numbered = files
        .iter()
        .map(|file| match split_number(&stem(file)) {
            (tag, Some(number)) => Ok((tag, number, file)),
            (_, None) => Err(format!(
                "{} has no frame number; a sequence is named <tag>_<number>.png",
                name(file)
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut strips = Vec::new();
    for (tag, members) in by_tag(numbered) {
        for pair in members.windows(2) {
            if pair[1].0 != pair[0].0 + 1 {
                return Err(format!(
                    "{tag} skips from {} to {}; a sequence has no gaps",
                    pair[0].0, pair[1].0
                ));
            }
        }
        let frames = members
            .into_iter()
            .map(|(_, file)| decode_image(file).map(ExportFrame::untimed))
            .collect::<Result<Vec<_>, _>>()?;
        strips.push(ExportStrip { tag, frames });
    }
    let mut export = AnimationExport::new(files.to_vec(), ExportFormat::Sequence, strips);
    unify(&mut export)?;
    Ok(export)
}

fn decode_gif(file: &Path) -> Result<AnimationExport, String> {
    let handle = File::open(file).map_err(|error| format!("{}: {error}", name(file)))?;
    let mut decoder = GifDecoder::new(BufReader::new(handle))
        .map_err(|error| format!("{}: {error}", name(file)))?;
    decoder
        .set_limits(limits())
        .map_err(|error| error.to_string())?;
    let frames = decoder
        .into_frames()
        .collect_frames()
        .map_err(|error| format!("{}: {error}", name(file)))?;
    animated(file, ExportFormat::Gif, frames, |numer, denom| {
        let ms = numer / denom.max(1);
        if ms == 0 { DEFAULT_MS } else { ms }
    })
}

fn decode_apng(file: &Path) -> Result<AnimationExport, String> {
    let handle = File::open(file).map_err(|error| format!("{}: {error}", name(file)))?;
    let mut decoder = PngDecoder::new(BufReader::new(handle))
        .map_err(|error| format!("{}: {error}", name(file)))?;
    decoder
        .set_limits(limits())
        .map_err(|error| error.to_string())?;
    let frames = decoder
        .apng()
        .map_err(|error| format!("{}: {error}", name(file)))?
        .into_frames()
        .collect_frames()
        .map_err(|error| format!("{}: {error}", name(file)))?;
    animated(file, ExportFormat::Apng, frames, |numer, denom| {
        let denom = denom.max(1);
        let ms = (numer * 2 + denom) / (denom * 2);
        if ms == 0 { DEFAULT_MS } else { ms }
    })
}

fn animated(
    file: &Path,
    kind: ExportFormat,
    frames: Vec<image::Frame>,
    ms_of: impl Fn(u32, u32) -> u32,
) -> Result<AnimationExport, String> {
    if frames.is_empty() {
        return Err(format!("{} has no frames", name(file)));
    }
    let frames = frames
        .into_iter()
        .map(|frame| {
            let (numer, denom) = frame.delay().numer_denom_ms();
            ExportFrame::new(frame.into_buffer(), Some(ms_of(numer, denom)))
        })
        .collect();
    let strip = ExportStrip {
        tag: stem(file),
        frames,
    };
    let mut export = AnimationExport::new(vec![file.to_path_buf()], kind, vec![strip]);
    unify(&mut export)?;
    Ok(export)
}

fn unify(export: &mut AnimationExport) -> Result<(), String> {
    let mut width = 0;
    let mut height = 0;
    let mut differ = false;
    for strip in &export.strips {
        for frame in &strip.frames {
            let (w, h) = frame.image.dimensions();
            if width != 0 && (w, h) != (width, height) {
                differ = true;
            }
            width = width.max(w);
            height = height.max(h);
        }
    }
    if width == 0 || height == 0 {
        return Err("the export has no frames".to_owned());
    }
    if width > MAX_FRAME_PX || height > MAX_FRAME_PX {
        return Err(format!(
            "frames of {width} × {height} are over the {MAX_FRAME_PX} px limit"
        ));
    }
    if differ {
        for strip in &mut export.strips {
            for frame in &mut strip.frames {
                let (w, h) = frame.image.dimensions();
                if (w, h) != (width, height) {
                    let mut canvas = RgbaImage::new(width, height);
                    let _ = canvas.copy_from(&frame.image, (width - w) / 2, height - h);
                    *frame = ExportFrame::new(canvas, frame.ms);
                }
            }
        }
        export
            .warnings
            .push("frames of different sizes were aligned at their feet".to_owned());
    }
    export.frame = (width, height);
    Ok(())
}

pub fn split_number(stem: &str) -> (String, Option<u64>) {
    let digits = stem
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .count();
    if digits == 0 {
        return (stem.to_owned(), None);
    }
    let (head, tail) = stem.split_at(stem.len() - digits);
    let number = tail.parse().ok();
    let tag = head.trim_end_matches(['_', '-', ' ', '/', '.']);
    if tag.is_empty() {
        return (stem.to_owned(), None);
    }
    (tag.to_owned(), number)
}

fn natural(text: &str) -> (String, u64) {
    let (tag, number) = split_number(text);
    (tag, number.unwrap_or(0))
}

fn image_beside(json: &Path, meta: &MetaJson) -> Result<PathBuf, String> {
    let image = meta
        .image
        .as_deref()
        .ok_or_else(|| format!("{} names no image; drop the sheet with it", name(json)))?;
    let beside = json.parent().unwrap_or(Path::new("")).join(image);
    if !beside.exists() {
        return Err(format!("{image} is not beside {}", name(json)));
    }
    Ok(beside)
}

fn by_tag<T>(items: impl IntoIterator<Item = (String, u64, T)>) -> Vec<(String, Vec<(u64, T)>)> {
    let mut groups: Vec<(String, Vec<(u64, T)>)> = Vec::new();
    for (tag, number, item) in items {
        match groups.iter_mut().find(|(seen, _)| *seen == tag) {
            Some((_, members)) => members.push((number, item)),
            None => groups.push((tag, vec![(number, item)])),
        }
    }
    for (_, members) in &mut groups {
        members.sort_by_key(|(number, _)| *number);
    }
    groups
}
