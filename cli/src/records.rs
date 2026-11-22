//! What a game server keeps, brought to the shapes this build reads: in a store on this machine,
//! or in one only another machine reaches, over a tunnel through ssh to it.

use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use clap::Subcommand;
use engine::core::content::LoadPurpose;
use engine::core::store::RecordStore;
use engine::core::store::RecordStoreAddress;
use engine::core::store::push::{self, PushedChange, PushedRecord};
use engine::systems::society::Society;
use url::Url;

use crate::folder;

#[derive(Subcommand)]
pub enum Records {
    /// Brings every record the store at STORE holds for GAME to the shapes this build reads, as
    /// its game server reads them, without migrating them: whatever of a record doesn't read is
    /// dropped (a field, a list entry, or the whole record when it can't be without what doesn't
    /// read), and what does read is kept. Tells what it would drop, and writes nothing unless
    /// `--write` is given. Run it while no game server uses the store.
    Push {
        game: PathBuf,
        /// The store's address, `postgres://…` or `sqlite://…`, as its game server is given it.
        /// With `--over`, the address the other machine reaches it at, its port named.
        #[arg(long)]
        store: String,
        /// Reaches the store over ssh to this destination, as `ssh` names one (`user@host`).
        #[arg(long)]
        over: Option<String>,
        /// Writes what the push makes of the records, rather than only telling it.
        #[arg(long)]
        write: bool,
    },
}

pub fn run(records: Records) {
    let result = match records {
        Records::Push {
            game,
            store,
            over,
            write,
        } => push_records(&game, &store, over.as_deref(), write),
    };
    if let Err(failure) = result {
        eprintln!("{failure}");
        std::process::exit(1);
    }
}

fn push_records(game: &Path, store: &str, over: Option<&str>, write: bool) -> Result<(), String> {
    let content = folder::load(game, "the game did not load", LoadPurpose::Play)
        .map_err(|report| report.to_string())?;
    let (_tunnel, address) = match over {
        Some(destination) => {
            let (tunnel, address) = Tunnel::open(destination, store)?;
            (Some(tunnel), address)
        }
        None => (None, RecordStoreAddress::new(store)),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("no runtime to reach the store on: {error}"))?;
    runtime.block_on(async {
        let store = RecordStore::open(&address)
            .await
            .map_err(|problem| format!("the store at {address} did not open: {problem}"))?;
        let kinds = Society::record_kinds(&content);
        let (changes, whole) = push::plan(&store, &kinds)
            .await
            .map_err(|problem| format!("the store could not be read: {problem}"))?;
        tell(&changes, whole);
        if changes.is_empty() {
            return Ok(());
        }
        if !write {
            println!("nothing written: run it again with --write to write it");
            return Ok(());
        }
        push::apply(&store, &changes)
            .await
            .map_err(|problem| format!("the push was not written: {problem}"))?;
        println!("written");
        Ok(())
    })
}

fn tell(changes: &[PushedChange], whole: usize) {
    let mut kept = 0;
    let mut gone = 0;
    for change in changes {
        let record = format!("{} {}", change.kind, change.id.0);
        match &change.pushed {
            PushedRecord::Whole => {}
            PushedRecord::Kept { dropped, .. } => {
                kept += 1;
                for dropped in dropped {
                    println!("{record}: drops {}: {}", dropped.at, dropped.why);
                }
            }
            PushedRecord::Gone(dropped) => {
                gone += 1;
                println!("{record}: goes, {}: {}", dropped.at, dropped.why);
            }
        }
    }
    println!("{whole} records read whole, {kept} are kept without what they drop, {gone} go");
}

/// A tunnel through ssh to another machine, from a free port of this one to the store's address
/// there; closed when dropped.
struct Tunnel {
    ssh: Child,
}

impl Tunnel {
    /// A tunnel over ssh to `destination` to the store `address` names, and the address that
    /// reaches the store through it.
    fn open(destination: &str, address: &str) -> Result<(Tunnel, RecordStoreAddress), String> {
        let mut url =
            Url::parse(address).map_err(|error| format!("the store's address: {error}"))?;
        let host = url
            .host_str()
            .ok_or("the store's address names no host to reach over ssh")?
            .to_owned();
        let port = url
            .port()
            .ok_or("the store's address names no port: name it to reach the store over ssh")?;
        let local = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|listener| listener.local_addr())
            .map_err(|error| format!("no free port for the tunnel: {error}"))?
            .port();
        let ssh = Command::new("ssh")
            .args([
                "-N",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "BatchMode=yes",
                "-L",
            ])
            .arg(format!("{}:{local}:{host}:{port}", Ipv4Addr::LOCALHOST))
            .arg(destination)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| format!("ssh did not start: {error}"))?;
        let mut tunnel = Tunnel { ssh };
        tunnel.wait_open(local, destination)?;
        url.set_ip_host(Ipv4Addr::LOCALHOST.into())
            .map_err(|()| "the store's address takes no other host")?;
        url.set_port(Some(local))
            .map_err(|()| "the store's address takes no other port")?;
        Ok((tunnel, RecordStoreAddress::new(url.as_str())))
    }

    fn wait_open(&mut self, local: u16, destination: &str) -> Result<(), String> {
        let since = Instant::now();
        while TcpStream::connect((Ipv4Addr::LOCALHOST, local)).is_err() {
            if let Ok(Some(status)) = self.ssh.try_wait() {
                return Err(format!(
                    "ssh to {destination} ended before the tunnel opened: {status}"
                ));
            }
            if since.elapsed() > OPEN_WITHIN {
                return Err(format!(
                    "the tunnel to {destination} did not open in {OPEN_WITHIN:?}"
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.ssh.kill();
        let _ = self.ssh.wait();
    }
}

const OPEN_WITHIN: Duration = Duration::from_secs(30);
