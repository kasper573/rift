#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod client;
mod protocol;
mod server;
mod wire;

pub use client::{
    ClientReplicationPlugin, ClientSystems, Remote, ReplicatedDespawn, ServerEntities, ServerLink,
};
pub use protocol::{
    AppMessageExt, AppReplicationExt, FromClient, MAX_COMPONENT_KINDS, Overwrite, Replicable,
    ToClient,
};
pub use server::{
    ClientLink, Interest, InterestInAll, Replicated, ReplicationInterest, Sender, Sending,
    ServerReplicationPlugin, ServerSystems,
};
