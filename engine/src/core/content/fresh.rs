use super::files::{name_of, parent_of};
use super::report::article;
use super::{ContentFile, ContentId, FolderReader};
use crate::core::assets::{AssetKind, AudioRef, ImageRef};

/// Where a content file is made fresh: the game around it, whose nearest files and media it names
/// wherever it must name some, and the path it is made at.
pub struct Nearby<'a> {
    reader: &'a FolderReader,
    path: &'a str,
}

impl<'a> Nearby<'a> {
    pub fn new(reader: &'a FolderReader, path: &'a str) -> Nearby<'a> {
        Nearby { reader, path }
    }

    /// The file of `Row` nearest the one made, by how few folders up from it their paths meet.
    pub fn file<Row: ContentFile>(&self) -> Result<ContentId<Row>, String> {
        let paths = self.reader.paths_of::<Row>().iter().map(String::as_str);
        self.nearest(paths)
            .and_then(|path| self.reader.id_of(path))
            .ok_or_else(|| {
                format!(
                    "it needs {} {}: make one first",
                    article(Row::TABLE),
                    Row::TABLE
                )
            })
    }

    /// The file of `Row` nearest the one made among those `fits`, or why there is none, as `wanted`
    /// names what fits.
    pub fn file_where<Row: ContentFile>(
        &self,
        wanted: &str,
        fits: impl Fn(&Row) -> bool,
    ) -> Result<ContentId<Row>, String> {
        let paths = self
            .reader
            .paths_of::<Row>()
            .iter()
            .map(String::as_str)
            .filter(|path| self.reader.read::<Row>(path).is_ok_and(|row| fits(&row)));
        self.nearest(paths)
            .and_then(|path| self.reader.id_of(path))
            .ok_or_else(|| format!("it needs {wanted}: make one first"))
    }

    /// The file `id` names, as it reads.
    pub fn read<Row: ContentFile>(&self, id: ContentId<Row>) -> Result<Row, String> {
        let path = self
            .reader
            .path_of(id)
            .ok_or("it names a file the game lacks")?;
        self.reader
            .read(path)
            .map_err(|_| format!("{path} doesn't read"))
    }

    pub fn image(&self) -> Result<ImageRef, String> {
        self.media(AssetKind::Image).map(ImageRef::new)
    }

    /// The nearest image of `size` pixels, for a picture its drawing takes at one size alone.
    pub fn image_sized(&self, (width, height): (u32, u32)) -> Result<ImageRef, String> {
        let images = self
            .reader
            .media()
            .iter()
            .map(String::as_str)
            .filter(|path| AssetKind::of(path) == Some(AssetKind::Image))
            .filter(|path| self.reader.image_size(path) == Some((width, height)));
        self.nearest(images)
            .map(|path| ImageRef::new(path.to_owned()))
            .ok_or_else(|| format!("it needs a {width} × {height} image: import one first"))
    }

    pub fn audio(&self) -> Result<AudioRef, String> {
        self.media(AssetKind::Audio).map(AudioRef::new)
    }

    /// What a player or designer calls the file by its own name: "Frost wolf" for
    /// `frost_wolf.npc.json`.
    pub fn name(&self) -> String {
        let words = name_of(self.path).replace('_', " ");
        let mut letters = words.trim().chars();
        match letters.next() {
            Some(first) => first.to_uppercase().chain(letters).collect(),
            None => "Unnamed".to_owned(),
        }
    }

    fn media(&self, kind: AssetKind) -> Result<String, String> {
        let of_kind = self
            .reader
            .media()
            .iter()
            .map(String::as_str)
            .filter(|path| AssetKind::of(path) == Some(kind));
        self.nearest(of_kind).map(str::to_owned).ok_or_else(|| {
            let kind: &'static str = kind.into();
            format!("it needs {} {kind} file: import one first", article(kind))
        })
    }

    fn nearest<'p>(&self, paths: impl Iterator<Item = &'p str>) -> Option<&'p str> {
        let here: Vec<&str> = parent_of(self.path)
            .split('/')
            .filter(|folder| !folder.is_empty())
            .collect();
        paths.min_by_key(|path| {
            let shared = here
                .iter()
                .zip(path.split('/'))
                .take_while(|(a, b)| **a == *b)
                .count();
            (here.len() - shared, *path)
        })
    }
}
