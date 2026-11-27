use std::sync::{Condvar, LazyLock, Mutex, PoisonError};

use bevy::prelude::default;
use bevy::render::RenderPlugin;
use bevy::render::renderer::{RenderDevice, RenderQueue, initialize_renderer};
use bevy::render::settings::{Backends, RenderCreation, RenderResources, WgpuSettings};

/// A device of the run's own, to one contract at a time and held for the whole contract: two
/// apps drawing on one device side by side stall each other's frames for good, so contracts on
/// devices of their own run side by side and the rest wait for one to come free.
pub fn exclusive() -> Gpu {
    let pool = &*POOL;
    let mut free = pool.free.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if let Some(device) = free.pop() {
            return Gpu { device };
        }
        free = pool
            .returned
            .wait(free)
            .unwrap_or_else(PoisonError::into_inner);
    }
}

/// A contract's turn on a device.
pub struct Gpu {
    device: usize,
}

impl Gpu {
    /// The renderer an app of the contract draws with.
    pub fn renderer(&self) -> RenderPlugin {
        RenderPlugin {
            render_creation: RenderCreation::Manual(Box::new(self.resources())),
            ..default()
        }
    }

    /// The device, for a contract driving the GPU without an app.
    pub fn device(&self) -> (RenderDevice, RenderQueue) {
        let RenderResources(device, queue, ..) = self.resources();
        (device, queue)
    }

    fn resources(&self) -> RenderResources {
        POOL.devices[self.device].clone()
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        POOL.free
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.device);
        POOL.returned.notify_one();
    }
}

/// How many contracts draw at once. Each device is made before the first contract draws and
/// holds memory of its own on the GPU, so the pool is kept to what drawing side by side gains.
const DEVICES: usize = 2;

struct Pool {
    devices: Vec<RenderResources>,
    free: Mutex<Vec<usize>>,
    returned: Condvar,
}

/// Every device made at once before any contract draws, and never dropped: the Vulkan loader
/// crashes naming an object on one thread while a device is made or dropped on another.
static POOL: LazyLock<Pool> = LazyLock::new(|| {
    let settings = WgpuSettings::default();
    let backends = settings.backends.unwrap_or(Backends::all());
    Pool {
        devices: (0..DEVICES)
            .map(|_| bevy::tasks::block_on(initialize_renderer(backends, None, &settings)))
            .collect(),
        free: Mutex::new((0..DEVICES).collect()),
        returned: Condvar::new(),
    }
});
