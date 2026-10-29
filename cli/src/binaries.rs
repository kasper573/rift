use std::path::PathBuf;

pub const GAME_SERVER: &str = "game-server";
pub const GAME_SERVER_PROFILING: &str = "game-server-profiling";
pub const DESKTOP_CLIENT: &str = "desktop-client";

/// The binary of `name` in the folder `rift` itself is in, as a release extracts them side by side.
pub fn beside(name: &str) -> PathBuf {
    let rift = std::env::current_exe().expect("rift's own path");
    let binary = rift
        .parent()
        .expect("rift's folder")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        binary.exists(),
        "{} is missing: rift runs the {name} beside it",
        binary.display()
    );
    binary
}
