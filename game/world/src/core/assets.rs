use include_dir::{Dir, include_dir};

pub use include_dir::File;

static ASSETS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../assets");

/// A logical, unresolved reference to a file in the assets folder, by fully qualified path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AssetRef(pub &'static str);

impl AssetRef {
    pub fn resolve(self) -> Option<&'static File<'static>> {
        ASSETS.get_file(normalize(self.0))
    }
}

fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}
