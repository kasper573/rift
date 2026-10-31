//! Server-authoritative replication for bevy.
//!
//! The server keeps one reliable, ordered stream to each client. Every send, it asks the app's
//! [`Interest`] which replicated entities each client sees, and tells each client only the
//! difference from what it already holds: entities that came into sight come whole, entities it
//! holds come by the components that changed or went away, and entities that left its sight are
//! despawned. Messages either way travel the same stream, after the entities of their send.
//!
//! The work runs on every core: each client's sight and difference is worked out in parallel,
//! and each changed component is serialized once, whoever is told of it.
//!
//! The crate moves bytes, not packets: an app's transport hands each [`ClientLink`] and the
//! [`ServerLink`] what arrives, and delivers what they have to send, reliably and in order.

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
    ClientLink, Everyone, Interest, Replicated, ReplicationInterest, ServerReplicationPlugin,
    ServerSystems,
};
