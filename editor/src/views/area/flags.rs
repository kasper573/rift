use std::collections::HashMap;

use engine::systems::area::scene::SceneNode;

use super::nodes;

/// A row's hide or lock: off, set on the row itself, or set by a folder above it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flag {
    #[default]
    Off,
    Own,
    Inherited,
}

impl Flag {
    pub fn on(self) -> bool {
        self != Flag::Off
    }

    fn of(own: bool, above: bool) -> Flag {
        match (own, above) {
            (true, _) => Flag::Own,
            (false, true) => Flag::Inherited,
            (false, false) => Flag::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowFlags {
    pub hidden: Flag,
    pub locked: Flag,
}

/// One scene shown in a tree: the bundle's own at the top, then the scene of each placement being
/// edited in place, under the row of the placement that opens it.
pub struct SceneLevel<'a> {
    /// The row the scene opens under, "" for the bundle's own.
    pub under: String,
    pub scene: &'a [SceneNode],
}

/// Which rows of a scene graph are hidden or locked, through the folders above them and a passing
/// Show only. A row's id is its node's pointer in the bundle's scene, or, inside a placement edited
/// in place, the placement's id, `/in`, and the pointer in its blueprint's scene.
#[derive(Clone, Debug, Default)]
pub struct SceneFlags {
    rows: HashMap<String, RowFlags>,
    /// Hidden and locked counts of the rows that aren't folders, in the bundle's own scene.
    pub hidden: usize,
    pub locked: usize,
}

impl SceneFlags {
    pub fn of(levels: &[SceneLevel], show_only: Option<&str>) -> SceneFlags {
        let mut flags = SceneFlags::default();
        for level in levels {
            let above = flags.rows.get(&level.under).copied().unwrap_or_default();
            let prefix = match level.under.as_str() {
                "" => String::new(),
                under => format!("{under}/in"),
            };
            flags.add(level.scene, &prefix, above, level.under.is_empty());
        }
        if let Some(only) = show_only {
            for (id, row) in &mut flags.rows {
                let related = id == only
                    || id.starts_with(&format!("{only}/"))
                    || only.starts_with(&format!("{id}/"));
                if !related && row.hidden == Flag::Off {
                    row.hidden = Flag::Inherited;
                }
            }
        }
        flags
    }

    pub fn row(&self, id: &str) -> RowFlags {
        self.rows.get(id).copied().unwrap_or_default()
    }

    pub fn hidden(&self, id: &str) -> bool {
        self.row(id).hidden.on()
    }

    pub fn locked(&self, id: &str) -> bool {
        self.row(id).locked.on()
    }

    /// Whether the map draws the module a placement brings by `path`: its node's pointer in the
    /// bundle's scene, then its pointer in each blueprint on the way to it. A blueprint's own flags
    /// count only where it is edited in place.
    pub fn keep(&self, path: &[&str]) -> bool {
        let Some((top, inner)) = path.split_first() else {
            return true;
        };
        let mut id = (*top).to_owned();
        if self.hidden(&id) {
            return false;
        }
        for pointer in inner {
            id = format!("{id}/in{pointer}");
            match self.rows.get(&id) {
                Some(row) if row.hidden.on() => return false,
                Some(_) => {}
                None => break,
            }
        }
        true
    }

    /// The hidden and locked counts as the header says them, when either is set.
    pub fn said(&self) -> Option<String> {
        let said: Vec<String> = [(self.hidden, "hidden"), (self.locked, "locked")]
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, what)| format!("{count} {what}"))
            .collect();
        (!said.is_empty()).then(|| said.join(", "))
    }

    /// Whether `other` hides the same rows, so a picture drawn for either draws the other.
    pub(crate) fn hides_alike(&self, other: &SceneFlags) -> bool {
        self.rows.len() == other.rows.len()
            && self.rows.iter().all(|(id, row)| {
                other
                    .rows
                    .get(id)
                    .is_some_and(|theirs| theirs.hidden.on() == row.hidden.on())
            })
    }

    pub(crate) fn any_hidden(&self) -> bool {
        self.rows.values().any(|row| row.hidden.on())
    }

    fn add(&mut self, scene: &[SceneNode], prefix: &str, above: RowFlags, own_scene: bool) {
        let mut stack: Vec<(String, RowFlags)> = Vec::new();
        nodes::walk(scene, &mut |pointer, node| {
            while let Some((folder, _)) = stack.last() {
                if pointer.starts_with(&format!("{folder}/")) {
                    break;
                }
                stack.pop();
            }
            let inherited = stack.last().map_or(above, |(_, flags)| *flags);
            let row = RowFlags {
                hidden: Flag::of(node.hidden(), inherited.hidden.on()),
                locked: Flag::of(node.locked(), inherited.locked.on()),
            };
            if own_scene && !nodes::is_folder(node) {
                self.hidden += usize::from(row.hidden.on());
                self.locked += usize::from(row.locked.on());
            }
            if nodes::is_folder(node) {
                stack.push((pointer.to_owned(), row));
            }
            self.rows.insert(format!("{prefix}{pointer}"), row);
        });
    }
}
