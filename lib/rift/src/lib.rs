// The derive macro emits `::rift::Wire` paths so it works in any crate — this alias makes
// them resolve inside rift itself.
extern crate self as rift;

pub mod app;
mod client;
mod cluster;
mod codec;
#[cfg(feature = "host")]
mod host;
mod link;
mod metrics;
mod server;
mod world;

pub use app::{App, Builder, Ctx, Feature, Resources, View};
pub use client::Client;
pub use cluster::{Cluster, Zone};
pub use codec::Wire;
#[cfg(feature = "host")]
pub use host::{Authenticator, TcpCluster};
pub use link::{Link, LinkStatus, Transport};
pub use metrics::Metrics;
pub use rift_derive::Wire;
pub use server::{Connection, Server, Session};
pub use world::{ClientId, Entity, Fast, Map, OwnerFn, Set, World};
