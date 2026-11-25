use super::{EventDef, EventKind, Reach, ReachDef};

#[derive(Clone, Default)]
pub struct MayDo {
    pub answers: Vec<EventKind>,
    pub reaches: Vec<Reach>,
    /// The reaches an event's flow comes to before anything on its way could stop it or ask:
    /// what that event surely does.
    pub sure: Vec<(EventKind, Reach)>,
    pub dialogue: Vec<String>,
    pub calls: Vec<String>,
    pub waits: bool,
}

impl MayDo {
    pub fn answers(&self, kind: EventKind) -> bool {
        self.answers.contains(&kind)
    }

    /// Every reach of the kind `R` the owner's scripts may have.
    pub fn reaching<R: ReachDef>(&self) -> impl Iterator<Item = &R> {
        self.reaches.iter().filter_map(Reach::downcast_ref::<R>)
    }

    /// Every reach of the kind `R` the owner's scripts surely have once an event of the kind `E`
    /// comes.
    pub fn surely_on<E: EventDef, R: ReachDef>(&self) -> impl Iterator<Item = &R> {
        self.sure
            .iter()
            .filter(|(kind, _)| *kind == EventKind::of::<E>())
            .filter_map(|(_, reach)| reach.downcast_ref::<R>())
    }

    /// What one owner's procedures may do between them, the events each answers included.
    pub fn owned<'a>(parts: impl IntoIterator<Item = &'a MayDo>) -> MayDo {
        let mut all = MayDo::default();
        for part in parts {
            all.absorb(part);
            all.answers.extend(part.answers.iter().copied());
            all.sure.extend(part.sure.iter().cloned());
        }
        all
    }

    /// Takes in what a called procedure may do; the events it answers stay its own.
    pub fn absorb(&mut self, other: &MayDo) {
        self.reaches.extend(other.reaches.iter().cloned());
        self.dialogue.extend(other.dialogue.iter().cloned());
        self.calls.extend(other.calls.iter().cloned());
        self.waits |= other.waits;
    }
}
