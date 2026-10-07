use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TableId<Row>(Row);

impl<Row> TableId<Row> {
    pub(crate) const fn new(row: Row) -> TableId<Row> {
        TableId(row)
    }

    pub(crate) fn row(self) -> Row {
        self.0
    }
}

impl<Row: fmt::Debug> fmt::Debug for TableId<Row> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<Row: FromStr> FromStr for TableId<Row> {
    type Err = Row::Err;

    fn from_str(name: &str) -> Result<TableId<Row>, Row::Err> {
        name.parse().map(TableId)
    }
}

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
        mod row {
            #[derive(
                Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
                serde::Serialize, serde::Deserialize, strum::EnumString,
            )]
            #[strum(ascii_case_insensitive)]
            pub enum Row {
                $first
                $(, $key)*
            }
        }

        pub type Id = $crate::core::table::TableId<row::Row>;

        pub static TABLE: &[$def] = &[
            $def $first_body,
            $($row $body,)*
        ];

        #[allow(non_upper_case_globals)]
        impl Id {
            $crate::table_row!($($first_mark)? $first);
            $($crate::table_row!($($mark)? $key);)*

            pub const VARIANTS: &'static [Id] = &[
                Id::new(row::Row::$first)
                $(, Id::new(row::Row::$key))*
            ];

            pub fn get(self) -> &'static $def {
                &TABLE[self.index()]
            }

            pub fn index(self) -> usize {
                self.row() as usize
            }
        }
    };
}

#[macro_export]
macro_rules! table_row {
    (expose $key:ident) => {
        pub const $key: Id = Id::new(row::Row::$key);
    };
    ($key:ident) => {
        #[allow(dead_code)]
        pub(super) const $key: Id = Id::new(row::Row::$key);
    };
}

pub use table;

pub fn parse_id<Row: FromStr>(
    name: &str,
    raw: Option<&str>,
    kind: &str,
) -> Result<TableId<Row>, String> {
    let raw = bevy_terminal::require_arg(name, raw)?;
    raw.parse()
        .map_err(|_| format!("`{raw}` is not a known {kind}"))
}
