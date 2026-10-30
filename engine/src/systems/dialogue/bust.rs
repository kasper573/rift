//! A bust: the portrait of one individual a conversation shows beside their lines, a face for every
//! generic expression and any of their own.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumString, VariantArray};

use crate::core::assets::{FileSize, ImageRef};
use crate::core::content::media::{VisiblePixelColumns, visible_columns};
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, ContentProblem, ContentRegistry, ErrorPlace,
    FileWriter, LoadStep, Loading, MISSING_FILE, Nearby, PartSpec, Parts, RowCheck, Shape,
};

pub type BustId = ContentId<BustDef>;

/// The faces of a bust, kept in its bundle under `faces/`: one `faces/<expression>.png` each.
#[derive(JsonSchema, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BustDef {
    #[serde(skip)]
    pub generic: GenericFaces,
    #[serde(skip)]
    pub own: Vec<OwnFace>,
}

#[derive(Clone)]
pub struct OwnFace {
    pub name: String,
    pub file: ImageRef,
}

#[derive(Clone, Default)]
pub struct GenericFaces {
    pub neutral: ImageRef,
    pub happy: ImageRef,
    pub sad: ImageRef,
    pub angry: ImageRef,
    pub surprised: ImageRef,
    pub thinking: ImageRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Expression {
    Generic(GenericExpression),
    Own(String),
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    VariantArray,
    AsRefStr,
    EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum GenericExpression {
    #[default]
    Neutral,
    Happy,
    Sad,
    Angry,
    Surprised,
    Thinking,
}

/// Busts carry wide transparent margins, so a bust is placed by its figure; taking the span over
/// all expressions keeps a new face from moving it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BustFigure {
    pub left: f32,
    pub right: f32,
}

// The client loads every bust at startup, so each face, and the whole cast, must stay a quick
// download.
pub const MAX_FACE_BYTES: FileSize = FileSize::kib(120);
pub const MAX_CAST_BYTES: FileSize = FileSize::kib(3 * 1024);

pub const BUST_FACES: PartSpec = PartSpec::Keyed {
    folder: "faces",
    exts: &["png"],
};

pub fn content(registry: &mut ContentRegistry) {
    registry
        .bundle::<BustDef>()
        .hook(LoadStep::Files, build_all);
}

impl BustDef {
    pub fn all(&self) -> impl Iterator<Item = &ImageRef> {
        GenericExpression::VARIANTS
            .iter()
            .map(|&expression| self.generic.of(expression))
            .chain(self.own.iter().map(|own| &own.file))
    }

    pub fn face(&self, face: &Expression) -> Option<&ImageRef> {
        match face {
            Expression::Generic(expression) => Some(self.generic.of(*expression)),
            Expression::Own(name) => self
                .own
                .iter()
                .find(|own| own.name == *name)
                .map(|own| &own.file),
        }
    }

    /// The face for `face`, or the neutral one when the bust has none of that name.
    pub fn face_or_neutral(&self, face: &Expression) -> &ImageRef {
        self.face(face)
            .unwrap_or_else(|| self.generic.of(GenericExpression::Neutral))
    }
}

impl GenericFaces {
    pub fn of(&self, expression: GenericExpression) -> &ImageRef {
        match expression {
            GenericExpression::Neutral => &self.neutral,
            GenericExpression::Happy => &self.happy,
            GenericExpression::Sad => &self.sad,
            GenericExpression::Angry => &self.angry,
            GenericExpression::Surprised => &self.surprised,
            GenericExpression::Thinking => &self.thinking,
        }
    }

    fn of_mut(&mut self, expression: GenericExpression) -> &mut ImageRef {
        match expression {
            GenericExpression::Neutral => &mut self.neutral,
            GenericExpression::Happy => &mut self.happy,
            GenericExpression::Sad => &mut self.sad,
            GenericExpression::Angry => &mut self.angry,
            GenericExpression::Surprised => &mut self.surprised,
            GenericExpression::Thinking => &mut self.thinking,
        }
    }
}

impl From<String> for Expression {
    fn from(name: String) -> Expression {
        match name.parse() {
            Ok(expression) => Expression::Generic(expression),
            Err(_) => Expression::Own(name),
        }
    }
}

impl From<Expression> for String {
    fn from(face: Expression) -> String {
        match face {
            Expression::Generic(expression) => expression.as_ref().to_owned(),
            Expression::Own(name) => name,
        }
    }
}

impl JsonSchema for Expression {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Expression")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = String::json_schema(generator);
        schema.insert("x-face".to_owned(), true.into());
        schema
    }

    fn inline_schema() -> bool {
        true
    }
}

impl Bundle for BustDef {
    const PARTS: &'static [PartSpec] = &[BUST_FACES];

    fn read_parts(&mut self, parts: &mut Parts) {
        let keyed = parts.keyed(BUST_FACES);
        for &expression in GenericExpression::VARIANTS {
            let part = BUST_FACES.named(expression.as_ref(), "png");
            if parts.has(&part) {
                *self.generic.of_mut(expression) = ImageRef::new(parts.path(&part));
            } else {
                parts.error(
                    "",
                    ErrorPlace::WholeFile,
                    ContentProblem::new(
                        MISSING_FILE,
                        format!("no {part}"),
                        "a face for every generic expression in faces/",
                    ),
                );
            }
        }
        self.own = keyed
            .iter()
            .filter(|(name, _)| name.parse::<GenericExpression>().is_err())
            .map(|(name, file)| OwnFace {
                name: (*name).to_owned(),
                file: ImageRef::new(parts.path(file)),
            })
            .collect();
        let measured: Vec<MeasuredFace> = keyed
            .iter()
            .map(|(_, file)| MeasuredFace {
                file,
                size: parts.image_size(file),
                weight: parts.weight(file),
            })
            .collect();
        for (file, problem) in face_problems(&measured) {
            parts.error(file, ErrorPlace::WholeFile, problem);
        }
    }

    fn write_parts(&self, _: &str, _: &mut FileWriter) -> std::io::Result<()> {
        Ok(())
    }
}

impl ContentFile for BustDef {
    const TABLE: &'static str = "bust";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(_: &Nearby) -> Result<BustDef, String> {
        Ok(BustDef::default())
    }
}

/// Where each bust's figure stands across its faces, by its id.
pub struct BustFigures(pub Vec<Option<BustFigure>>);

impl BustFigures {
    /// The figure of `bust`, or the whole width when its faces show nothing.
    pub fn of(content: &Content, bust: BustId) -> BustFigure {
        content
            .built::<BustFigures>()
            .0
            .get(bust.index())
            .copied()
            .flatten()
            .unwrap_or(BustFigure {
                left: 0.0,
                right: 1.0,
            })
    }
}

/// Measures every bust's figure, and holds the cast of busts to its download budget.
async fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let rows = loading.rows::<BustDef>(&content);
    let folder = loading.folder();
    let measured: Vec<(BustId, Option<BustFigure>, FileSize)> = loading
        .each(rows, |(id, def)| {
            let files: Vec<Vec<u8>> = def
                .all()
                .filter_map(|face| folder.read(&face.path()).ok())
                .collect();
            let spans: Vec<VisiblePixelColumns> = files
                .iter()
                .filter_map(|bytes| visible_columns(bytes).ok().flatten())
                .collect();
            let left = spans
                .iter()
                .map(|span| span.first as f32 / span.image_width as f32);
            let right = spans
                .iter()
                .map(|span| (span.last + 1) as f32 / span.image_width as f32);
            let figure = left
                .reduce(f32::min)
                .zip(right.reduce(f32::max))
                .map(|(left, right)| BustFigure { left, right });
            let weight = FileSize(files.iter().map(Vec::len).sum());
            (id, figure, weight)
        })
        .await;
    let mut figures: Vec<Option<BustFigure>> = content.ids::<BustDef>().map(|_| None).collect();
    let mut total = 0;
    let mut over = false;
    for (id, figure, weight) in measured {
        figures[id.index()] = figure;
        total += weight.0;
        if FileSize(total) > MAX_CAST_BYTES && !std::mem::replace(&mut over, true) {
            let mut check = RowCheck::default();
            check.error(
                "",
                "cast too heavy",
                format!(
                    "the busts up to {} weigh {}",
                    id.path(&content),
                    FileSize(total)
                ),
                format!("at most {MAX_CAST_BYTES} for every bust in the game"),
            );
            loading.row_errors(id, check);
        }
    }
    loading.built(BustFigures(figures));
}

struct MeasuredFace<'a> {
    file: &'a str,
    size: Option<(u32, u32)>,
    weight: Option<FileSize>,
}

/// What a bust's faces must be to each other: one size, each light enough to download.
fn face_problems<'a>(faces: &[MeasuredFace<'a>]) -> Vec<(&'a str, ContentProblem)> {
    let mut problems = Vec::new();
    let mut first: Option<(&str, (u32, u32))> = None;
    for face in faces {
        if let Some(weight) = face.weight
            && weight > MAX_FACE_BYTES
        {
            problems.push((
                face.file,
                ContentProblem::new(
                    "face too heavy",
                    format!("{} weighs {weight}", face.file),
                    format!("at most {MAX_FACE_BYTES}"),
                ),
            ));
        }
        let Some(own) = face.size else {
            continue;
        };
        match first {
            None => first = Some((face.file, own)),
            Some((earlier, (w, h))) if (w, h) != own => {
                problems.push((
                    face.file,
                    ContentProblem::new(
                        "face sizes differ",
                        format!(
                            "{} is {} × {}, {earlier} is {w} × {h}",
                            face.file, own.0, own.1
                        ),
                        "every face of one bust at one size",
                    ),
                ));
                break;
            }
            Some(_) => {}
        }
    }
    problems
}
