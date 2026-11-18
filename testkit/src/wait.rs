use bevy_app::App;

/// Steps `app` until `done`, however long the threads its tasks run on take: what a test waits
/// for is a state, so how it ends never depends on how fast the machine is. A wait that never
/// ends is a hang the test run's own limit stops.
pub fn until(app: &mut App, mut done: impl FnMut(&mut App) -> bool) {
    loop {
        app.update();
        if done(app) {
            return;
        }
        std::thread::yield_now();
    }
}
