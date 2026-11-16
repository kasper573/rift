use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Where a server's records are kept: `postgres://…` for a database server, `sqlite://…` for a
/// file.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecordStoreAddress(String);

impl RecordStoreAddress {
    pub fn new(address: impl Into<String>) -> RecordStoreAddress {
        RecordStoreAddress(address.into())
    }

    /// The file at `path`, made when it isn't there.
    pub fn file(path: &Path) -> RecordStoreAddress {
        RecordStoreAddress(format!("sqlite://{}?mode=rwc", path.display()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The address without the password it may hold, so it can be named in a log.
impl fmt::Display for RecordStoreAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let named = self.0.split_once("://").and_then(|(scheme, rest)| {
            let (user, host) = rest.rsplit_once('@')?;
            let (name, _) = user.split_once(':')?;
            Some(format!("{scheme}://{name}@{host}"))
        });
        f.write_str(named.as_deref().unwrap_or(&self.0))
    }
}

impl fmt::Debug for RecordStoreAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RecordStoreAddress({self})")
    }
}
