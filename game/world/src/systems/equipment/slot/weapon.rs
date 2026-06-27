use super::Slot;

#[derive(Clone, Copy)]
pub struct WeaponSlot;

inventory::submit! {
    &WeaponSlot as &dyn Slot
}

impl Slot for WeaponSlot {
    fn name(&self) -> &'static str {
        "weapon"
    }
    fn label(&self) -> &'static str {
        "Weapon"
    }
}
