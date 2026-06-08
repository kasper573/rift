use std::any::{Any, TypeId};

use crate::{ClientId, Connection, Entity, Map, OwnerFn, Server, Set, Wire, World};

/// Plain functions so features hold no state; registered order is run order.
pub type Feature = fn(&mut Builder);

// The server is borrowed per call so the transport keeps owning it.
pub struct App {
    sched: Schedule,
    res: Resources,
    events: Events,
    time: f32,
}

impl App {
    pub fn new(features: &[Feature]) -> Self {
        let mut builder = Builder {
            sched: Schedule::default(),
        };
        for &feature in features {
            feature(&mut builder);
        }
        Self {
            sched: builder.sched,
            res: Resources::default(),
            events: Events::default(),
            time: 0.0,
        }
    }

    pub fn insert_resource<T: 'static + Send>(&mut self, value: T) {
        self.res.insert(value);
    }

    pub fn zone_of(&self, world: &World, entity: Entity) -> Option<u32> {
        self.sched.zone_of.and_then(|f| f(world, entity))
    }
    pub fn owner_of(&self, world: &World, entity: Entity) -> Option<ClientId> {
        self.sched.owner_of.and_then(|f| f(world, entity))
    }

    pub fn start(&mut self, server: &mut Server) {
        let App {
            sched, res, events, ..
        } = self;
        if let Some(owner_of) = sched.owner_of {
            server.owned_by(owner_of);
        }
        for &hook in &sched.start {
            hook(&mut ctx(server, res, events, 0.0, 0.0));
        }
    }

    pub fn tick(&mut self, server: &mut Server, dt: f32) {
        self.time += dt;
        let App {
            sched, res, events, ..
        } = self;
        let time = self.time;

        for connection in server.drain_connections() {
            let (hooks, client) = match connection {
                Connection::On(client) => (&sched.connect, client),
                Connection::Off(client) => (&sched.disconnect, client),
                Connection::Migrate(client, entity) => {
                    for &hook in &sched.migrate {
                        hook(&mut ctx(server, res, events, time, dt), client, entity);
                    }
                    continue;
                }
            };
            for &hook in hooks {
                hook(&mut ctx(server, res, events, time, dt), client);
            }
        }

        for &hook in &sched.intents {
            hook(&mut ctx(server, res, events, time, dt));
        }
        for &hook in &sched.systems {
            hook(&mut ctx(server, res, events, time, dt));
        }

        // Subscribers may emit more events; the guard bounds runaway cascades.
        let mut guard = 0;
        loop {
            let batch = std::mem::take(&mut events.queue);
            if batch.is_empty() || guard >= 16 {
                break;
            }
            guard += 1;
            for (type_id, event) in &batch {
                if let Some(handlers) = sched.events.get(type_id) {
                    for handler in handlers {
                        handler(&mut ctx(server, res, events, time, dt), event.as_ref());
                    }
                }
            }
        }

        // No `see` hooks means the game hasn't opted into visibility filtering at all.
        if sched.see.is_empty() {
            return;
        }
        for client in server.client_ids().to_vec() {
            let mut visible = Set::default();
            {
                let view = View {
                    world: &server.world,
                    res: &*res,
                };
                for &hook in &sched.see {
                    hook(&view, client, &mut visible);
                }
            }
            server.set_visibility(client, Some(visible));
        }
    }
}

pub struct Builder {
    sched: Schedule,
}

impl Builder {
    pub fn start(&mut self, hook: StartHook) {
        self.sched.start.push(hook);
    }
    pub fn connect(&mut self, hook: ConnHook) {
        self.sched.connect.push(hook);
    }
    pub fn disconnect(&mut self, hook: ConnHook) {
        self.sched.disconnect.push(hook);
    }
    pub fn intent(&mut self, hook: IntentHook) {
        self.sched.intents.push(hook);
    }
    pub fn system(&mut self, hook: SystemHook) {
        self.sched.systems.push(hook);
    }
    pub fn see(&mut self, hook: SeeHook) {
        self.sched.see.push(hook);
    }
    /// Runs after a migrated entity is rebuilt on this shard, its state already moved.
    pub fn migrate(&mut self, hook: MigrateHook) {
        self.sched.migrate.push(hook);
    }
    /// Game logic just sets the zone component; the cluster moves the entity to that shard.
    pub fn shard_by(&mut self, zone_of: ZoneFn) {
        self.sched.zone_of = Some(zone_of);
    }
    pub fn owned_by(&mut self, owner_of: OwnerFn) {
        self.sched.owner_of = Some(owner_of);
    }

    pub fn on<E: 'static>(&mut self, handler: fn(&mut Ctx, &E)) {
        self.sched
            .events
            .entry(TypeId::of::<E>())
            .or_default()
            .push(Box::new(move |ctx, any| {
                if let Some(event) = any.downcast_ref::<E>() {
                    handler(ctx, event);
                }
            }));
    }

    pub fn replicate<T: Wire + 'static>(&mut self) {
        self.sched.start.push(replicate_hook::<T>);
    }

    /// Like [`Self::replicate`], but the component reaches only its entity's owner
    /// (resolved through [`Self::owned_by`]); other clients never receive it.
    pub fn replicate_to_owner<T: Wire + 'static>(&mut self) {
        self.sched.start.push(replicate_to_owner_hook::<T>);
    }
}

pub struct Ctx<'a> {
    pub server: &'a mut Server,
    pub res: &'a mut Resources,
    pub events: &'a mut Events,
    pub time: f32,
    pub dt: f32,
}

pub struct View<'a> {
    pub world: &'a World,
    pub res: &'a Resources,
}

#[derive(Default)]
pub struct Resources(Map<TypeId, Box<dyn Any + Send>>);

impl Resources {
    pub fn insert<T: 'static + Send>(&mut self, value: T) {
        self.0.insert(TypeId::of::<T>(), Box::new(value));
    }
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.0
            .get(&TypeId::of::<T>())
            .and_then(|any| any.downcast_ref())
    }
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.0
            .get_mut(&TypeId::of::<T>())
            .and_then(|any| any.downcast_mut())
    }
}

#[derive(Default)]
pub struct Events {
    queue: Vec<(TypeId, Box<dyn Any + Send>)>,
}

impl Events {
    pub fn emit<E: 'static + Send>(&mut self, event: E) {
        self.queue.push((TypeId::of::<E>(), Box::new(event)));
    }
}

pub type StartHook = fn(&mut Ctx);
pub type ConnHook = fn(&mut Ctx, ClientId);
pub type IntentHook = fn(&mut Ctx);
pub type SystemHook = fn(&mut Ctx);
pub type SeeHook = fn(&View, ClientId, &mut Set<Entity>);
pub type MigrateHook = fn(&mut Ctx, ClientId, Entity);
pub type ZoneFn = fn(&World, Entity) -> Option<u32>;
type EventHook = Box<dyn Fn(&mut Ctx, &dyn Any) + Send>;

#[derive(Default)]
struct Schedule {
    start: Vec<StartHook>,
    connect: Vec<ConnHook>,
    disconnect: Vec<ConnHook>,
    intents: Vec<IntentHook>,
    systems: Vec<SystemHook>,
    see: Vec<SeeHook>,
    migrate: Vec<MigrateHook>,
    zone_of: Option<ZoneFn>,
    owner_of: Option<OwnerFn>,
    events: Map<TypeId, Vec<EventHook>>,
}

fn replicate_hook<T: Wire + 'static>(ctx: &mut Ctx) {
    ctx.server.replicate::<T>();
}

fn replicate_to_owner_hook<T: Wire + 'static>(ctx: &mut Ctx) {
    ctx.server.replicate_to_owner::<T>();
}

fn ctx<'a>(
    server: &'a mut Server,
    res: &'a mut Resources,
    events: &'a mut Events,
    time: f32,
    dt: f32,
) -> Ctx<'a> {
    Ctx {
        server,
        res,
        events,
        time,
        dt,
    }
}
