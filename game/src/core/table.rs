#[macro_export]
macro_rules! table {
    (
        #![expose]
        $($key:ident : $row:ident $body:tt),+
        $(,)?
    ) => {
        $crate::table! { $(#[expose] $key : $row $body),+ }
    };
    (
        $(#[$first_mark:ident])? $first:ident : $def:ident $first_body:tt
        $(, $(#[$mark:ident])? $key:ident : $row:ident $body:tt)*
        $(,)?
    ) => {
        pub type Id = $crate::core::content::ContentId<$def>;

        const KEYS: &[$crate::core::content::KeyBuf] = &[
            $crate::core::content::key_of(stringify!($first))
            $(, $crate::core::content::key_of(stringify!($key)))*
        ];

        #[allow(non_upper_case_globals)]
        impl Id {
            $crate::table_row!($($first_mark)? $first);
            $($crate::table_row!($($mark)? $key);)*
        }

        pub static TABLE: &[$def] = &[
            $def $first_body,
            $($row $body,)*
        ];

        pub fn rows() -> Vec<(String, $def)> {
            KEYS.iter()
                .zip(TABLE)
                .map(|(key, row)| (key.as_str().to_owned(), row.clone()))
                .collect()
        }
    };
}

#[macro_export]
macro_rules! table_row {
    (expose $key:ident) => {
        pub const $key: Id = Id::new($crate::core::content::sorted_index(
            KEYS,
            &$crate::core::content::key_of(stringify!($key)),
        ));
    };
    ($key:ident) => {
        #[allow(dead_code)]
        pub(super) const $key: Id = Id::new($crate::core::content::sorted_index(
            KEYS,
            &$crate::core::content::key_of(stringify!($key)),
        ));
    };
}

pub use table;
