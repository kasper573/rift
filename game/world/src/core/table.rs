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
        )]
        pub enum Id {
            $first
            $(, $key)*
        }

        pub static TABLE: ::std::sync::LazyLock<::std::collections::HashMap<Id, $def>> =
            ::std::sync::LazyLock::new(|| {
                ::std::collections::HashMap::from([
                    (Id::$first, $def $first_body),
                    $((Id::$key, $row $body),)*
                ])
            });

        impl Id {
            pub fn get(self) -> &'static $def {
                &TABLE[&self]
            }
            pub fn index(self) -> usize {
                self as usize
            }
            pub fn name(self) -> &'static str {
                match self {
                    Id::$first => stringify!($first),
                    $(Id::$key => stringify!($key),)*
                }
            }
            pub fn by_name(name: &str) -> Result<Id, String> {
                <Id as ::serde::Deserialize>::deserialize(
                    ::serde::de::value::StrDeserializer::<::serde::de::value::Error>::new(name),
                )
                .map_err(|error| error.to_string())
            }
        }
    };
}

pub use table;
