use engine::core::units::Bytes;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// The memory a running process holds resident, as the system sees it.
pub(crate) fn resident(process: u32) -> Bytes {
    let pid = Pid::from_u32(process);
    let mut system = System::new().expect("the system's processes to read");
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing().with_memory(),
    );
    Bytes(
        system
            .process(pid)
            .expect("the process is running")
            .memory(),
    )
}
