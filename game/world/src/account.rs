//! The player account: its authenticated [`Identity`] and the authorization [`Role`]s it carries.

pub mod identity;
pub mod role;

pub use identity::Identity;
pub use role::Role;
