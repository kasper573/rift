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
