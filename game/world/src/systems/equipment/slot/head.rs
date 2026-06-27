use super::Slot;

#[derive(Clone, Copy)]
pub struct HeadSlot;

inventory::submit! {
    &HeadSlot as &dyn Slot
}

impl Slot for HeadSlot {
    fn name(&self) -> &'static str {
        "head"
    }
    fn label(&self) -> &'static str {
        "Head"
    }
}
