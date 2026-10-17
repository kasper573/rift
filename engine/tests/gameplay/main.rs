mod casting;
mod conversation;
mod effects;
mod hotbar;
mod skills;
mod travel;

use std::path::Path;

use driver::{Pace, Playtest, PlaytestSetup};

// A playtest of the example content, leaving what it recorded under the test's name should it fail.
fn playtest() -> Playtest {
    let thread = std::thread::current();
    let test = thread.name().expect("a test thread named after its test");
    Playtest::start(PlaytestSetup {
        server_binary: env!("CARGO_BIN_EXE_game-server").into(),
        client_binary: env!("CARGO_BIN_EXE_desktop-client").into(),
        content: Path::new(env!("CARGO_MANIFEST_DIR")).join("../example_game"),
        folder: Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("gameplay")
            .join(std::process::id().to_string())
            .join(test.replace("::", ".")),
        pace: Pace::Brisk,
        seed: None,
        recorded: true,
    })
}
