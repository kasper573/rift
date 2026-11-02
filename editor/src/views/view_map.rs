use std::collections::HashMap;

/// The state of the open views of one kind, by the key of the row each shows. A view is made when
/// its window first shows, before anything reads or changes it.
pub struct ViewMap<V>(HashMap<String, V>);

impl<V> Default for ViewMap<V> {
    fn default() -> ViewMap<V> {
        ViewMap(HashMap::new())
    }
}

impl<V> ViewMap<V> {
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    pub fn insert(&mut self, key: &str, view: V) {
        self.0.insert(key.to_owned(), view);
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.0.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.0.get_mut(key)
    }

    /// The view of `key`, made by `make` when it isn't yet.
    pub fn view_or_insert_with(&mut self, key: &str, make: impl FnOnce() -> V) -> &mut V {
        self.0.entry(key.to_owned()).or_insert_with(make)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.0.values_mut()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn view(&self, key: &str) -> &V {
        self.0
            .get(key)
            .unwrap_or_else(|| panic!("the view of {key} was read before it was made"))
    }

    pub fn view_mut(&mut self, key: &str) -> &mut V {
        self.0
            .get_mut(key)
            .unwrap_or_else(|| panic!("the view of {key} was changed before it was made"))
    }
}

impl<V: Default> ViewMap<V> {
    pub fn ensure_default(&mut self, key: &str) {
        self.0.entry(key.to_owned()).or_default();
    }

    /// The view of `key`, made as a fresh view when it isn't yet.
    pub fn view_or_default(&mut self, key: &str) -> &mut V {
        self.0.entry(key.to_owned()).or_default()
    }
}
