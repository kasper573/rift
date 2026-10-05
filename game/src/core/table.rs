#[macro_export]
macro_rules! table {
    (
        $first:ident : $def:ident $first_body:tt
        $(, $key:ident : $row:ident $body:tt)*
        $(,)?
    ) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
            serde::Serialize, serde::Deserialize,
            strum::VariantArray, strum::EnumString,
        )]
        pub enum Id {
            $first
            $(, $key)*
        }

        pub static TABLE: &[$def] = &[
            $def $first_body,
            $($row $body,)*
        ];

        impl Id {
            pub fn get(self) -> &'static $def {
                &TABLE[self as usize]
            }
            pub fn index(self) -> usize {
                self as usize
            }
        }
    };
}

pub use table;

pub fn parse_id<Id: strum::VariantArray + std::fmt::Debug + Copy>(
    name: &str,
    raw: Option<&str>,
    kind: &str,
) -> Result<Id, String> {
    let raw = bevy_terminal::require_arg(name, raw)?;
    Id::VARIANTS
        .iter()
        .copied()
        .find(|id| format!("{id:?}").eq_ignore_ascii_case(raw))
        .ok_or_else(|| format!("`{raw}` is not a known {kind}"))
}
