use std::sync::Arc;

use bevy_ecs::prelude::{Component, Entity};
use bevy_ecs::world::{EntityWorldMut, World};
use bevy_math::Vec2;
use bevy_ui::prelude::{Button, ImageNode, Node, Text};

/// Reads a value off the live world each render — the dynamic leaves of a [`View`]
/// (`<Show when=…>`, `<For each=…>`, `{ |w| … }` text).
pub(crate) type Reader<T> = Arc<dyn Fn(&World) -> T + Send + Sync>;
/// An entity-aware side effect run against the world when an element is clicked, hovered, dragged,
/// mounts, or unmounts. Behaviors receive their own entity; user handlers usually ignore it.
pub(crate) type Act = Arc<dyn Fn(&mut World, Entity) + Send + Sync>;
/// A drag side effect, additionally carrying the pointer delta for this frame.
pub(crate) type DragAct = Arc<dyn Fn(&mut World, Entity, Vec2) + Send + Sync>;
/// An idempotent setter applied to an element's entity every render.
pub(crate) type Apply = Arc<dyn Fn(&mut EntityWorldMut) + Send + Sync>;

pub(crate) const TAG_NODE: u64 = 0;
pub(crate) const TAG_TEXT: u64 = 1;
pub(crate) const TAG_BUTTON: u64 = 2;
pub(crate) const TAG_IMAGE: u64 = 3;

/// A declarative description of a piece of UI. Built once with the runtime constructors (or the
/// [`view!`](bevy_view_macro::view) macro) and reconciled into `bevy_ui` entities each frame.
#[derive(Clone)]
pub struct View(pub(crate) ViewKind);

#[derive(Clone)]
pub(crate) enum ViewKind {
    Empty,
    Element(Box<Element>),
    Show {
        when: Reader<bool>,
        body: Box<View>,
    },
    Each(Reader<Vec<(u64, View)>>),
    Fragment(Vec<View>),
    /// Renders `body` into the registered [`outlet`] for `kind` instead of in place — overlay content
    /// portals to an app-placed sink so draw order comes from outlet placement, not z-index.
    Portal {
        kind: PortalKind,
        body: Box<View>,
    },
    /// A context boundary that produces no entity but gives its subtree a stable instance id, so a
    /// trigger and its portaled content (an overlay) share state without a hand-written key.
    Provide {
        body: Box<View>,
    },
    /// Control-flow gating keyed by the enclosing [`Provide`] instance: `body` is included only while
    /// `test` reads true for that instance. Overlays use this to mount content only while open.
    Gate {
        test: GateTest,
        body: Box<View>,
    },
}

/// Identifies a portal destination: a [`portal`] routes its body to the [`outlet`] declared with the
/// same kind. Overlays reserve their own kinds; games may use any value.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PortalKind(pub u64);

/// A stable identity for a [`Provide`] subtree, derived from its position in the view tree, linking a
/// trigger and its portaled content without a hand-written key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InstanceId(pub(crate) u64);

/// Tests whether a [`Gate`] should currently render its body, given the enclosing instance and the
/// host entity whose children are being collected (so the test can read that entity's context).
pub(crate) type GateTest = Arc<dyn Fn(&World, InstanceId, Entity) -> bool + Send + Sync>;

/// Marks an [`outlet`] entity; the reconciler leaves its children to the per-frame portal flush
/// rather than reconciling them from the view.
#[derive(Component, Clone, Copy)]
pub(crate) struct PortalSink(pub(crate) PortalKind);

/// A single UI element: a base bundle inserted on mount, idempotent per-render setters, optional
/// dynamic text, fan-out lifecycle/pointer handlers, and child views.
#[derive(Clone)]
pub struct Element {
    pub(crate) tag: u64,
    pub(crate) base: Apply,
    pub(crate) apply: Vec<Apply>,
    pub(crate) text: Option<Reader<String>>,
    pub(crate) click: Vec<Act>,
    pub(crate) drag: Vec<DragAct>,
    pub(crate) drag_end: Vec<Act>,
    pub(crate) over: Vec<Act>,
    pub(crate) out: Vec<Act>,
    pub(crate) mount: Vec<Act>,
    pub(crate) cleanup: Vec<Act>,
    pub(crate) children: Vec<View>,
}

/// A `bevy_ui` container element backed by a [`Node`]. Style it with bareword attributes (which set
/// `Node` fields) or [`Element::insert`].
pub fn node() -> Element {
    Element::new(TAG_NODE, |entity| {
        entity.insert(Node::default());
    })
}

/// A text element whose content is fixed.
pub fn text(content: impl Into<String>) -> Element {
    let content = content.into();
    dyn_text(move |_| content.clone())
}

/// A text element whose content is read off the world every render.
pub fn dyn_text<F>(content: F) -> Element
where
    F: Fn(&World) -> String + Send + Sync + 'static,
{
    let mut element = Element::new(TAG_TEXT, |entity| {
        entity.insert(Text::default());
    });
    element.text = Some(Arc::new(content));
    element
}

/// The `bevy_ui` [`Button`] primitive: a clickable [`Node`].
pub fn button() -> Element {
    Element::new(TAG_BUTTON, |entity| {
        entity.insert((Node::default(), Button));
    })
}

/// The `bevy_ui` [`ImageNode`] primitive. Pass a `Handle<Image>` (or any `Into<ImageNode>`). The source
/// is re-applied every render (like [`text`]'s content), so a re-rendered image follows a changed
/// source rather than keeping the one it mounted with — only the `Node` is base-once.
pub fn image(source: impl Into<ImageNode>) -> Element {
    let image = source.into();
    Element::new(TAG_IMAGE, |entity| {
        entity.insert(Node::default());
    })
    .attr(move |entity| {
        entity.insert(image.clone());
    })
}

/// Includes `body` only while `when` reads true; toggling it mounts/unmounts the subtree as real
/// control flow (its `on_cleanup` fires when it leaves), not a visibility toggle.
pub fn show<F, V>(when: F, body: V) -> View
where
    F: Fn(&World) -> bool + Send + Sync + 'static,
    V: Into<View>,
{
    View(ViewKind::Show {
        when: Arc::new(when),
        body: Box::new(body.into()),
    })
}

/// The inverse of [`show`]: includes `body` only while `when` reads false.
pub fn hide<F, V>(when: F, body: V) -> View
where
    F: Fn(&World) -> bool + Send + Sync + 'static,
    V: Into<View>,
{
    show(move |world| !when(world), body)
}

/// Renders one child per item, reusing entities across renders by `key`. Items keep their entity
/// identity and retained component state as long as their key is stable.
pub fn each<I, FI, FK, FT, V>(items: FI, key: FK, template: FT) -> View
where
    I: 'static,
    FI: Fn(&World) -> Vec<I> + Send + Sync + 'static,
    FK: Fn(&I) -> u64 + Send + Sync + 'static,
    FT: Fn(&I) -> V + Send + Sync + 'static,
    V: Into<View>,
{
    View(ViewKind::Each(Arc::new(move |world| {
        items(world)
            .iter()
            .map(|item| (key(item), template(item).into()))
            .collect()
    })))
}

/// Renders `body` into the [`outlet`] registered for `kind`, wherever in the tree that outlet sits.
/// The body keeps stable identity across renders and unmounts (firing `on_cleanup`) when removed.
pub fn portal(kind: PortalKind, body: impl Into<View>) -> View {
    View(ViewKind::Portal {
        kind,
        body: Box::new(body.into()),
    })
}

/// A destination for [`portal`]s of `kind`. Place it where you want that content to render — its
/// position in the tree decides paint order. Its children are owned entirely by the portal flush.
pub fn outlet(kind: PortalKind) -> Element {
    Element::new(TAG_NODE, move |entity| {
        entity.insert((Node::default(), PortalSink(kind)));
    })
}

/// Wraps `body` in a context boundary that gives the subtree a stable instance id — the primitive a
/// component library uses to link a trigger to its portaled content without a hand-written key.
pub fn boundary(body: impl Into<View>) -> View {
    View(ViewKind::Provide {
        body: Box::new(body.into()),
    })
}

/// Includes `body` only while `test` reads true for the enclosing [`boundary`] instance — control flow
/// (mount/unmount), not visibility. `test` receives the instance id and the host entity whose children
/// `body` sits among, so it can consult that entity's [`context`](crate::context) (e.g. an item's value).
pub fn gate<F>(test: F, body: impl Into<View>) -> View
where
    F: Fn(&World, InstanceId, Entity) -> bool + Send + Sync + 'static,
{
    View(ViewKind::Gate {
        test: Arc::new(test),
        body: Box::new(body.into()),
    })
}

impl View {
    /// Renders nothing.
    pub fn empty() -> View {
        View(ViewKind::Empty)
    }

    /// Groups sibling views without introducing a wrapper element.
    pub fn fragment(views: impl IntoIterator<Item = View>) -> View {
        View(ViewKind::Fragment(views.into_iter().collect()))
    }
}

/// A composable element decorator: installs components and handlers onto an element. Behaviors (like
/// `draggable`) hand these out; markup applies them with `use={…}`, repeatably.
pub struct Bind(Box<dyn FnOnce(Element) -> Element>);

impl Bind {
    pub fn new<F>(decorate: F) -> Bind
    where
        F: FnOnce(Element) -> Element + 'static,
    {
        Bind(Box::new(decorate))
    }
}

impl Element {
    fn new<F>(tag: u64, base: F) -> Element
    where
        F: Fn(&mut EntityWorldMut) + Send + Sync + 'static,
    {
        Element {
            tag,
            base: Arc::new(base),
            apply: Vec::new(),
            text: None,
            click: Vec::new(),
            drag: Vec::new(),
            drag_end: Vec::new(),
            over: Vec::new(),
            out: Vec::new(),
            mount: Vec::new(),
            cleanup: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Runs an idempotent setter on the element's entity every render. Use this to write the
    /// specific fields the view owns; fields it never touches (e.g. a drag system's position)
    /// survive reconciliation.
    pub fn attr<F>(mut self, setter: F) -> Element
    where
        F: Fn(&mut EntityWorldMut) + Send + Sync + 'static,
    {
        self.apply.push(Arc::new(setter));
        self
    }

    /// Inserts (and on later renders re-inserts) a bundle on the element's entity. Convenient for
    /// components the view fully owns; prefer bareword `Node` attributes for partial `Node` updates
    /// so retained fields survive.
    pub fn insert<B>(self, bundle: B) -> Element
    where
        B: bevy_ecs::bundle::Bundle + Clone,
    {
        self.attr(move |entity| {
            entity.insert(bundle.clone());
        })
    }

    /// Applies a composable behavior (see [`Bind`]). Repeatable; binds apply in order.
    pub fn bind(self, bind: Bind) -> Element {
        (bind.0)(self)
    }

    pub fn on_click<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World) + Send + Sync + 'static,
    {
        self.click.push(Arc::new(move |world, _| handler(world)));
        self
    }

    /// Click handler that also receives the element's own entity — for behaviors.
    pub fn on_click_with<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World, Entity) + Send + Sync + 'static,
    {
        self.click.push(Arc::new(handler));
        self
    }

    pub fn on_drag<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World, Vec2) + Send + Sync + 'static,
    {
        self.drag
            .push(Arc::new(move |world, _, delta| handler(world, delta)));
        self
    }

    /// Drag handler that also receives the element's own entity — for behaviors.
    pub fn on_drag_with<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World, Entity, Vec2) + Send + Sync + 'static,
    {
        self.drag.push(Arc::new(handler));
        self
    }

    pub fn on_drag_end<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World) + Send + Sync + 'static,
    {
        self.drag_end.push(Arc::new(move |world, _| handler(world)));
        self
    }

    pub fn on_drag_end_with<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World, Entity) + Send + Sync + 'static,
    {
        self.drag_end.push(Arc::new(handler));
        self
    }

    pub fn on_over<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World) + Send + Sync + 'static,
    {
        self.over.push(Arc::new(move |world, _| handler(world)));
        self
    }

    pub fn on_over_with<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World, Entity) + Send + Sync + 'static,
    {
        self.over.push(Arc::new(handler));
        self
    }

    pub fn on_out<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World) + Send + Sync + 'static,
    {
        self.out.push(Arc::new(move |world, _| handler(world)));
        self
    }

    pub fn on_out_with<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World, Entity) + Send + Sync + 'static,
    {
        self.out.push(Arc::new(handler));
        self
    }

    pub fn on_mount<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World) + Send + Sync + 'static,
    {
        self.mount.push(Arc::new(move |world, _| handler(world)));
        self
    }

    pub fn on_mount_with<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World, Entity) + Send + Sync + 'static,
    {
        self.mount.push(Arc::new(handler));
        self
    }

    pub fn on_cleanup<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World) + Send + Sync + 'static,
    {
        self.cleanup.push(Arc::new(move |world, _| handler(world)));
        self
    }

    pub fn on_cleanup_with<F>(mut self, handler: F) -> Element
    where
        F: Fn(&mut World, Entity) + Send + Sync + 'static,
    {
        self.cleanup.push(Arc::new(handler));
        self
    }

    /// Distinguishes this element's type for reconciliation: when the element at a slot changes tag
    /// across renders, its entity is despawned and a fresh one spawned rather than mutated.
    pub fn tag(mut self, tag: u64) -> Element {
        self.tag = tag;
        self
    }

    /// Shows `icon` while the pointer is over this element. Read the resulting desired cursor with
    /// [`hovered_cursor`](crate::hovered_cursor); an element without a cursor never changes it.
    pub fn cursor(self, icon: crate::CursorIcon) -> Element {
        self.insert(crate::HoverCursor(icon))
    }

    pub fn child(mut self, child: impl Into<View>) -> Element {
        self.children.push(child.into());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = View>) -> Element {
        self.children.extend(children);
        self
    }
}

impl From<Element> for View {
    fn from(element: Element) -> View {
        View(ViewKind::Element(Box::new(element)))
    }
}
