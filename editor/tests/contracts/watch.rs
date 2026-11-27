use std::time::{Duration, Instant};

use editor::project::watch::{FolderChange, FolderWatcher};

#[test]
fn reading_a_watched_folder_is_no_change_and_writing_one_is() {
    let folder = tempfile::tempdir().expect("a folder");
    let (read, written) = (
        folder.path().join("hat.item.json"),
        folder.path().join("cap.item.json"),
    );
    std::fs::write(&read, b"{}").expect("written");
    std::fs::write(&written, b"{}").expect("written");
    let mut watcher = FolderWatcher::on(folder.path(), QUIET);

    std::fs::read(&read).expect("read");
    std::fs::read_dir(folder.path())
        .expect("listed")
        .for_each(drop);
    std::fs::write(&written, b"{ \"name\": \"Cap\" }").expect("rewritten");

    match settled(&mut watcher) {
        Some(FolderChange::Files(files)) => assert_eq!(files, ["cap.item.json"]),
        Some(FolderChange::Unknown) => panic!("the watcher lost track"),
        None => panic!("a write is a change"),
    }
}

const QUIET: Duration = Duration::from_millis(50);

/// The first change the watcher tells of, given ample time to tell one.
fn settled(watcher: &mut FolderWatcher) -> Option<FolderChange> {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if let Some(change) = watcher.poll() {
            return Some(change);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}
