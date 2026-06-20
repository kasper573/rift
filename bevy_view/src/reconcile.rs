use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::prelude::{Click, Drag, DragEnd, Out, Over, Pointer};

use crate::view::{Act, DragAct, Element, InstanceId, PortalKind, PortalSink, View, ViewKind};

#[derive(Component)]
pub struct ViewNode;

/// Stamped onto every element under a [`boundary`](crate::boundary) so component libraries can query
/// which overlay instance an entity belongs to.
#[derive(Component, Clone, Copy)]
pub struct Instance(pub(crate) InstanceId);

impl Instance {
    pub fn id(&self) -> InstanceId {
        self.0
    }
}

#[derive(Component)]
struct Managed {
    path: Path,
    tag: u64,
}

/// The live handlers for an entity, refreshed every render so the latest tree's closures win without
/// stacking. Every event fans out: all handlers (element and `use=` behaviors) run in order.
#[derive(Component, Default)]
pub(crate) struct ViewState {
    pub(crate) click: Vec<Act>,
    pub(crate) drag: Vec<DragAct>,
    pub(crate) drag_end: Vec<Act>,
    pub(crate) over: Vec<Act>,
    pub(crate) out: Vec<Act>,
    pub(crate) mount: Vec<Act>,
    pub(crate) cleanup: Vec<Act>,
}

type Path = Vec<Seg>;

#[derive(Clone, PartialEq, Eq, Hash)]
enum Seg {
    Index(u32),
    Key(u64),
    Dup(u64, u32),
}

#[derive(Clone, Copy)]
struct Cx {
    instance: Option<InstanceId>,
    host: Entity,
}

struct Desired {
    path: Path,
    instance: Option<InstanceId>,
    element: Element,
}

/// A subtree siphoned out to be reconciled under its outlet. `seed` distinguishes portals sharing an
/// instance by their source location, so they reconcile to distinct paths rather than colliding.
struct PortalItem {
    kind: PortalKind,
    instance: InstanceId,
    seed: u64,
    body: View,
}

pub fn render(world: &mut World, host: Entity, view: View) {
    let mut desired = Vec::new();
    let mut portals = Vec::new();
    let mut cx = Cx {
        instance: None,
        host,
    };
    collect(
        world,
        &mut Path::new(),
        &mut cx,
        view,
        &mut desired,
        &mut portals,
    );
    reconcile_level(world, host, desired, &mut portals);
    flush_portals(world, portals);
}

pub fn activate_click(world: &mut World, entity: Entity) {
    run_acts(world, entity, |state| &state.click);
}

pub fn activate_drag(world: &mut World, entity: Entity, delta: Vec2) {
    let handlers = world
        .get::<ViewState>(entity)
        .map(|state| state.drag.clone())
        .unwrap_or_default();
    for handler in handlers {
        handler(world, entity, delta);
    }
}

pub fn activate_drag_end(world: &mut World, entity: Entity) {
    run_acts(world, entity, |state| &state.drag_end);
}

pub fn activate_over(world: &mut World, entity: Entity) {
    run_acts(world, entity, |state| &state.over);
}

pub fn activate_out(world: &mut World, entity: Entity) {
    run_acts(world, entity, |state| &state.out);
}

pub fn instance_of(world: &World, entity: Entity) -> Option<InstanceId> {
    world.get::<Instance>(entity).map(|instance| instance.0)
}

fn run_acts(world: &mut World, entity: Entity, select: impl Fn(&ViewState) -> &Vec<Act>) {
    let handlers = world
        .get::<ViewState>(entity)
        .map(|state| select(state).clone())
        .unwrap_or_default();
    for handler in handlers {
        handler(world, entity);
    }
}

fn collect(
    world: &World,
    prefix: &mut Path,
    cx: &mut Cx,
    view: View,
    out: &mut Vec<Desired>,
    portals: &mut Vec<PortalItem>,
) {
    match view.0 {
        ViewKind::Empty => {}
        ViewKind::Element(element) => out.push(Desired {
            path: prefix.clone(),
            instance: cx.instance,
            element: *element,
        }),
        ViewKind::Fragment(views) => {
            for (index, view) in views.into_iter().enumerate() {
                prefix.push(Seg::Index(index as u32));
                collect(world, prefix, cx, view, out, portals);
                prefix.pop();
            }
        }
        ViewKind::Show { when, body } => {
            if when(world) {
                collect(world, prefix, cx, *body, out, portals);
            }
        }
        ViewKind::Each(items) => {
            let mut seen: HashMap<u64, u32> = HashMap::new();
            for (key, view) in items(world) {
                let occurrence = seen.entry(key).or_insert(0);
                let seg = if *occurrence == 0 {
                    Seg::Key(key)
                } else {
                    Seg::Dup(key, *occurrence)
                };
                *occurrence += 1;
                prefix.push(seg);
                collect(world, prefix, cx, view, out, portals);
                prefix.pop();
            }
        }
        ViewKind::Provide { body } => {
            let saved = cx.instance;
            // The host entity distinguishes structurally identical sibling boundaries (e.g. several
            // overlays together) so they don't share an instance.
            cx.instance = Some(InstanceId(hash_host(cx.host, prefix)));
            collect(world, prefix, cx, *body, out, portals);
            cx.instance = saved;
        }
        ViewKind::Portal { kind, body } => {
            let seed = hash_path(prefix);
            let instance = cx.instance.unwrap_or(InstanceId(seed));
            portals.push(PortalItem {
                kind,
                instance,
                seed,
                body: *body,
            });
        }
        ViewKind::Gate { test, body } => {
            if let Some(instance) = cx.instance
                && test(world, instance, cx.host)
            {
                collect(world, prefix, cx, *body, out, portals);
            }
        }
    }
}

fn reconcile_level(
    world: &mut World,
    parent: Entity,
    desired: Vec<Desired>,
    portals: &mut Vec<PortalItem>,
) {
    let existing: Vec<Entity> = world
        .get::<Children>(parent)
        .map(|children| children.iter().collect())
        .unwrap_or_default();
    let mut by_path: HashMap<Path, Entity> = HashMap::new();
    for &child in &existing {
        if let Some(managed) = world.get::<Managed>(child) {
            by_path.insert(managed.path.clone(), child);
        }
    }

    let mut ordered = Vec::with_capacity(desired.len());
    for item in desired {
        // Reuse only if the tag AND instance match; a slot switching between structurally different
        // views (e.g. plain node vs. Provide-wrapped) must remount, not graft.
        let reuse = by_path.get(&item.path).copied().filter(|&entity| {
            world.get::<Managed>(entity).map(|m| m.tag) == Some(item.element.tag)
                && instance_of(world, entity) == item.instance
        });
        let entity = match reuse {
            Some(entity) => entity,
            None => {
                if let Some(&stale) = by_path.get(&item.path) {
                    despawn_view(world, stale);
                }
                spawn_managed(world, parent, &item.path, &item.element, item.instance)
            }
        };
        apply_element(world, entity, item.element, item.instance, portals);
        ordered.push(entity);
    }

    let surviving: std::collections::HashSet<Entity> = ordered.iter().copied().collect();
    for &child in &existing {
        if world.get::<Managed>(child).is_some() && !surviving.contains(&child) {
            despawn_view(world, child);
        }
    }
    world.entity_mut(parent).replace_children(&ordered);
}

fn spawn_managed(
    world: &mut World,
    parent: Entity,
    path: &Path,
    element: &Element,
    instance: Option<InstanceId>,
) -> Entity {
    // Insert ViewNode last so the mount observer sees a fully-formed entity with Instance set.
    let mut entity = world.spawn((
        Managed {
            path: path.clone(),
            tag: element.tag,
        },
        view_state(element),
        ChildOf(parent),
    ));
    if let Some(instance) = instance {
        entity.insert(Instance(instance));
    }
    (element.base)(&mut entity);
    entity.insert(ViewNode);
    entity.id()
}

fn apply_element(
    world: &mut World,
    entity: Entity,
    element: Element,
    instance: Option<InstanceId>,
    portals: &mut Vec<PortalItem>,
) {
    {
        let mut entity_mut = world.entity_mut(entity);
        entity_mut.insert(view_state(&element));
        if let Some(instance) = instance {
            entity_mut.insert(Instance(instance));
        }
        for setter in &element.apply {
            setter(&mut entity_mut);
        }
    }
    if let Some(reader) = &element.text {
        let content = reader(world);
        if let Some(mut text) = world.get_mut::<bevy_ui::prelude::Text>(entity)
            && text.0 != content
        {
            text.0 = content;
        }
    }

    // Portal sinks' children are flushed separately, not reconciled from the view.
    if world.get::<PortalSink>(entity).is_some() {
        return;
    }

    let mut desired = Vec::new();
    let mut prefix = Path::new();
    let mut cx = Cx {
        instance,
        host: entity,
    };
    for (index, child) in element.children.into_iter().enumerate() {
        prefix.push(Seg::Index(index as u32));
        collect(world, &mut prefix, &mut cx, child, &mut desired, portals);
        prefix.pop();
    }
    reconcile_level(world, entity, desired, portals);
}

fn flush_portals(world: &mut World, initial: Vec<PortalItem>) {
    let outlets = find_all_outlets(world);
    if outlets.is_empty() {
        return;
    }
    let mut by_kind: HashMap<PortalKind, Vec<PortalItem>> = HashMap::new();
    for item in initial {
        by_kind.entry(item.kind).or_default().push(item);
    }
    // Reconcile to a fixpoint; reconciling one outlet can reveal nested portals. Guard bounds
    // pathological nesting.
    let mut guard = 0;
    loop {
        guard += 1;
        let mut discovered = Vec::new();
        for &(outlet, kind) in &outlets {
            let mut desired = Vec::new();
            if let Some(items) = by_kind.get(&kind) {
                for item in items {
                    let mut cx = Cx {
                        instance: Some(item.instance),
                        host: outlet,
                    };
                    let mut prefix = vec![Seg::Key(item.instance.0), Seg::Key(item.seed)];
                    collect(
                        world,
                        &mut prefix,
                        &mut cx,
                        item.body.clone(),
                        &mut desired,
                        &mut discovered,
                    );
                }
            }
            reconcile_level(world, outlet, desired, &mut discovered);
        }
        let mut grew = false;
        for item in discovered {
            let bucket = by_kind.entry(item.kind).or_default();
            if !bucket
                .iter()
                .any(|existing| existing.instance == item.instance)
            {
                bucket.push(item);
                grew = true;
            }
        }
        if !grew || guard >= 16 {
            break;
        }
    }
}

fn find_all_outlets(world: &mut World) -> Vec<(Entity, PortalKind)> {
    world
        .query::<(Entity, &PortalSink)>()
        .iter(world)
        .map(|(entity, sink)| (entity, sink.0))
        .collect()
}

fn hash_path(path: &Path) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

/// Hashes host + path so sibling boundaries get distinct, render-stable instances.
fn hash_host(host: Entity, path: &Path) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    host.hash(&mut hasher);
    path.hash(&mut hasher);
    hasher.finish()
}

fn despawn_view(world: &mut World, entity: Entity) {
    world.entity_mut(entity).despawn();
}

fn view_state(element: &Element) -> ViewState {
    ViewState {
        click: element.click.clone(),
        drag: element.drag.clone(),
        drag_end: element.drag_end.clone(),
        over: element.over.clone(),
        out: element.out.clone(),
        mount: element.mount.clone(),
        cleanup: element.cleanup.clone(),
    }
}

#[derive(Component, Clone)]
pub struct ViewRoot(Arc<dyn Fn(&World) -> View + Send + Sync>);

impl ViewRoot {
    pub fn new<F>(builder: F) -> ViewRoot
    where
        F: Fn(&World) -> View + Send + Sync + 'static,
    {
        ViewRoot(Arc::new(builder))
    }
}

pub(crate) fn render_roots(world: &mut World) {
    let roots: Vec<(Entity, ViewRoot)> = world
        .query::<(Entity, &ViewRoot)>()
        .iter(world)
        .map(|(entity, root)| (entity, root.clone()))
        .collect();
    // Portals accumulate and flush once per frame, so outlets reconcile all roots' content together.
    let mut portals = Vec::new();
    for (host, root) in roots {
        let view = (root.0)(world);
        let mut desired = Vec::new();
        let mut cx = Cx {
            instance: None,
            host,
        };
        collect(
            world,
            &mut Path::new(),
            &mut cx,
            view,
            &mut desired,
            &mut portals,
        );
        reconcile_level(world, host, desired, &mut portals);
    }
    flush_portals(world, portals);
}

pub(crate) fn on_view_added(
    add: On<Add, ViewNode>,
    states: Query<&ViewState>,
    mut commands: Commands,
) {
    let entity = add.entity;
    if let Ok(state) = states.get(entity)
        && !state.mount.is_empty()
    {
        let handlers = state.mount.clone();
        commands.queue(move |world: &mut World| run_all(world, entity, &handlers));
    }
}

pub(crate) fn on_view_removed(
    remove: On<Remove, ViewNode>,
    states: Query<&ViewState>,
    mut commands: Commands,
) {
    // Capture handlers now; the queued command runs after despawn.
    let entity = remove.entity;
    if let Ok(state) = states.get(entity)
        && !state.cleanup.is_empty()
    {
        let handlers = state.cleanup.clone();
        commands.queue(move |world: &mut World| run_all(world, entity, &handlers));
    }
}

fn run_all(world: &mut World, entity: Entity, handlers: &[Act]) {
    for handler in handlers {
        handler(world, entity);
    }
}

pub(crate) fn on_view_click(click: On<Pointer<Click>>, mut commands: Commands) {
    let entity = click.entity;
    commands.queue(move |world: &mut World| activate_click(world, entity));
}

pub(crate) fn on_view_drag(
    mut drag: On<Pointer<Drag>>,
    states: Query<&ViewState>,
    mut commands: Commands,
) {
    let entity = drag.entity;
    if states.get(entity).is_ok_and(|state| !state.drag.is_empty()) {
        let delta = drag.delta;
        commands.queue(move |world: &mut World| activate_drag(world, entity, delta));
        drag.propagate(false);
    }
}

pub(crate) fn on_view_drag_end(
    mut drag: On<Pointer<DragEnd>>,
    states: Query<&ViewState>,
    mut commands: Commands,
) {
    let entity = drag.entity;
    if states
        .get(entity)
        .is_ok_and(|state| !state.drag_end.is_empty())
    {
        commands.queue(move |world: &mut World| activate_drag_end(world, entity));
        drag.propagate(false);
    }
}

pub(crate) fn on_view_over(over: On<Pointer<Over>>, mut commands: Commands) {
    let entity = over.entity;
    commands.queue(move |world: &mut World| activate_over(world, entity));
}

pub(crate) fn on_view_out(out: On<Pointer<Out>>, mut commands: Commands) {
    let entity = out.entity;
    commands.queue(move |world: &mut World| activate_out(world, entity));
}
