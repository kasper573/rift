use std::sync::Arc;

use bevy_ecs::prelude::{Component, Entity};
use bevy_ecs::world::{EntityWorldMut, World};
use bevy_math::Vec2;
use bevy_ui::prelude::{Button, ImageNode, Node, Text};

pub(crate) type Reader<T> = Arc<dyn Fn(&World) -> T + Send + Sync>;
pub(crate) type Act = Arc<dyn Fn(&mut World, Entity) + Send + Sync>;
pub(crate) type DragAct = Arc<dyn Fn(&mut World, Entity, Vec2) + Send + Sync>;
pub(crate) type Apply = Arc<dyn Fn(&mut EntityWorldMut) + Send + Sync>;

pub(crate) const TAG_NODE: u64 = 0;
pub(crate) const TAG_TEXT: u64 = 1;
pub(crate) const TAG_BUTTON: u64 = 2;
pub(crate) const TAG_IMAGE: u64 = 3;

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
    /// Renders into the registered [`outlet`] for `kind` instead of in place — draw order comes from
    /// outlet placement, not z-index.
    Portal {
        kind: PortalKind,
        body: Box<View>,
    },
    /// A context boundary that gives its subtree a stable instance id, linking a trigger and its
    /// portaled content without a hand-written key.
    Provide {
        body: Box<View>,
    },
    Gate {
        test: GateTest,
        body: Box<View>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PortalKind(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InstanceId(pub(crate) u64);

pub(crate) type GateTest = Arc<dyn Fn(&World, InstanceId, Entity) -> bool + Send + Sync>;

#[derive(Component, Clone, Copy)]
pub(crate) struct PortalSink(pub(crate) PortalKind);

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

pub fn node() -> Element {
    Element::new(TAG_NODE, |entity| {
        entity.insert(Node::default());
    })
}

pub fn text(content: impl Into<String>) -> Element {
    let content = content.into();
    dyn_text(move |_| content.clone())
}

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

pub fn button() -> Element {
    Element::new(TAG_BUTTON, |entity| {
        entity.insert((Node::default(), Button));
    })
}

/// The source is re-applied every render so a re-rendered image follows a changed source; only the
/// `Node` is base-once.
pub fn image(source: impl Into<ImageNode>) -> Element {
    let image = source.into();
    Element::new(TAG_IMAGE, |entity| {
        entity.insert(Node::default());
    })
    .attr(move |entity| {
        entity.insert(image.clone());
    })
}

/// Toggles mount/unmount as real control flow, not a visibility toggle.
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

pub fn hide<F, V>(when: F, body: V) -> View
where
    F: Fn(&World) -> bool + Send + Sync + 'static,
    V: Into<View>,
{
    show(move |world| !when(world), body)
}

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

pub fn portal(kind: PortalKind, body: impl Into<View>) -> View {
    View(ViewKind::Portal {
        kind,
        body: Box::new(body.into()),
    })
}

pub fn outlet(kind: PortalKind) -> Element {
    Element::new(TAG_NODE, move |entity| {
        entity.insert((Node::default(), PortalSink(kind)));
    })
}

pub fn boundary(body: impl Into<View>) -> View {
    View(ViewKind::Provide {
        body: Box::new(body.into()),
    })
}

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
    pub fn empty() -> View {
        View(ViewKind::Empty)
    }

    pub fn fragment(views: impl IntoIterator<Item = View>) -> View {
        View(ViewKind::Fragment(views.into_iter().collect()))
    }
}

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

    /// Use this to write the specific fields the view owns; retained fields survive reconciliation.
    pub fn attr<F>(mut self, setter: F) -> Element
    where
        F: Fn(&mut EntityWorldMut) + Send + Sync + 'static,
    {
        self.apply.push(Arc::new(setter));
        self
    }

    /// Prefer bareword `Node` attributes for partial updates so retained fields survive.
    pub fn insert<B>(self, bundle: B) -> Element
    where
        B: bevy_ecs::bundle::Bundle + Clone,
    {
        self.attr(move |entity| {
            entity.insert(bundle.clone());
        })
    }

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

    /// Element at a slot with a different tag is despawned and respawned, not mutated.
    pub fn tag(mut self, tag: u64) -> Element {
        self.tag = tag;
        self
    }

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
