/// Define a normalized data table. The row keys become a `pub enum Id`, the rows a
/// `pub static TABLE: HashMap<Id, Def>`, and `Id::get()` resolves a row. The id lives once — as the
/// key — so it can't disagree with itself, every reference (`Id::Goblin`) is checked at compile time,
/// and `Id` derives serde so rows ride the wire as their key name.
///
/// ```ignore
/// table! {
///     Goblin: NpcDef { display_name: "Goblin", ai: &Aggressive, .. },
///     Orc:    NpcDef { .. },
/// }
/// // -> pub enum Id { Goblin, Orc }, pub static TABLE: HashMap<Id, NpcDef>, Id::Goblin.get()
/// ```
#[macro_export]
macro_rules! table {
    (
        $first:ident : $def:ident { $($first_field:tt)* }
        $(, $key:ident : $row:ident { $($field:tt)* })*
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
                    (Id::$first, $def { $($first_field)* }),
                    $((Id::$key, $row { $($field)* }),)*
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
