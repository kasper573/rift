/// The JSON pointer of a field of a serialized type, checked against the type: `field!(ModelDef,
/// hitbox.radius)` is `"/hitbox/radius"`, and naming a field the type doesn't have
/// doesn't compile.
#[macro_export]
macro_rules! field {
    ($ty:ty, $($field:ident).+) => {{
        let _ = |row: &$ty| {
            let _ = &row.$($field).+;
        };
        concat!($("/", stringify!($field)),+)
    }};
}

/// An effect's outputs, each named and held in a field of the effect: `outputs` and
/// `outputs_mut` of `EffectDef`.
#[macro_export]
macro_rules! outputs {
    ($($name:expr => $field:ident),+ $(,)?) => {
        fn outputs(&self) -> Vec<(&'static str, &$crate::core::script::Arrow)> {
            vec![$(($name, &self.$field)),+]
        }
        fn outputs_mut(&mut self) -> Vec<(&'static str, &mut $crate::core::script::Arrow)> {
            vec![$(($name, &mut self.$field)),+]
        }
    };
}

pub mod core;
pub mod systems;
