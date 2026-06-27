/// Define a normalized data table. The row keys become a `pub enum Id`, the rows a
/// `pub static TABLE: HashMap<Id, Def>`, and `Id::get()` resolves a row. The id lives once — as the
/// key — so it can't disagree with itself, every reference (`Id::Goblin`) is checked at compile time,
/// and `Id` derives serde so rows ride the wire as their key name.
///
/// Rows can be written as struct literals, taking the value type from each row:
///
/// ```ignore
/// table! {
///     Goblin: NpcDef { display_name: "Goblin", ai: &Aggressive, .. },
///     Orc:    NpcDef { .. },
/// }
/// ```
///
/// Or built from an expression, with the value type stated once — useful when rows are produced by a
/// function rather than a literal:
///
/// ```ignore
/// table! { ActorModel {
///     Orc: load(AssetRef("models/orc.tsx")),
///     Bat: load(AssetRef("models/bat.tsx")),
/// }}
/// ```
#[macro_export]
macro_rules! table {
    ( $def:ty { $($key:ident : $value:expr),* $(,)? } ) => {
        $crate::table!(@build $def; $($key => $value),*);
    };
    (
        $first:ident : $def:ident { $($first_field:tt)* }
        $(, $key:ident : $row:ident { $($field:tt)* })*
        $(,)?
    ) => {
        $crate::table!(@build $def;
            $first => $def { $($first_field)* }
            $(, $key => $row { $($field)* })*
        );
    };
    (@build $def:ty; $($key:ident => $value:expr),* $(,)?) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
            serde::Serialize, serde::Deserialize,
        )]
        pub enum Id {
            $($key),*
        }

        pub static TABLE: ::std::sync::LazyLock<::std::collections::HashMap<Id, $def>> =
            ::std::sync::LazyLock::new(|| {
                ::std::collections::HashMap::from([
                    $((Id::$key, $value),)*
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
