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
    AppMessageExt, AppReplicationExt, ClientMessage, ClientMessageRefusal, FromClient,
    MAX_COMPONENT_KINDS, Overwrite, ReadClientMessage, Replicable, ToClient, client_message_schema,
    over_the_wire, read_client_message, write_client_message,
};
pub use server::{
    ClientLink, Interest, InterestInAll, Replicated, ReplicationInterest, Sender, Sending,
    ServerReplicationPlugin, ServerSystems,
};
