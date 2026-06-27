use super::Slot;

#[derive(Clone, Copy)]
pub struct OffhandSlot;

inventory::submit! {
    &OffhandSlot as &dyn Slot
}

impl Slot for OffhandSlot {
    fn name(&self) -> &'static str {
        "offhand"
    }
    fn label(&self) -> &'static str {
        "Offhand"
    }
}
