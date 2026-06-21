# `ui` crate — mistakes I made

A paper trail of the concrete coding mistakes I made while rewriting the `ui` crate from the
`bevy_view` reconciler onto retained `bevy_ui`, and while chasing pixel-parity afterwards. Each
one is a place I wrote something that compiled and then did the wrong thing — silently, at
runtime, or only on one surface — with how it surfaced and how I corrected it.

### `UiPlugin` clobbered a theme the consumer had already set

I wrote `UiPlugin::build` to call `app.insert_resource(theme::default_theme())` unconditionally.
An app that set the light theme *before* adding the plugin silently got the dark default back. It
only showed on large theme-sensitive surfaces, so it hid for a while. I fixed it by inserting the
default only when no `Theme` resource exists yet. The root cause was treating the theme as one
global resource and letting the plugin overwrite app config it should have deferred to.

### I gave `popover()` a `Node` but `tooltip()` none, so composing the popover panicked

`tooltip()` deliberately returns no `Node` — its trigger node carries the layout so floating
content anchors to the trigger. I made `popover()`, the same kind of anchored overlay, return
`(Node, Open, Dismissable)` *with* a `Node`. Wrapping the popover in a positioning `Node` then
panicked at runtime: *"Bundle … has duplicate components."* I fixed it by dropping the popover's
own `Node`. There was no rule for which component free-fns own their `Node`, and getting it wrong
compiles and only blows up at runtime.

### Node-less component roots silently detached whole subtrees (B0004)

Because `tooltip()`/`popover()` carry no `Node`, nesting one between `Node` entities (`Node` →
marker (no `Node`) → `popover()` (no `Node`) → trigger `Node`) made Bevy emit
`warning[B0004]: Node entity has a parent without Node` and **the entire subtree stopped laying
out — nothing under it rendered.** No panic; the scene just came up blank. I had built the
gallery's edged tooltip/popover scenes exactly that way and they rendered nothing until I wrapped
the roots in `Node::default()`. A non-`Node` entity sitting between `Node`s is a layout landmine
that only warns.

### I omitted a required overlay part and got a silently-broken modal

A working dialog needs `dialog()` + `dialog_modal()` (the portal that re-parents content to the
full-screen overlay host) + `dialog_scrim()` + `dialog_content()`. I left a composition missing
`dialog_modal()`: it compiled, ran, didn't panic, and rendered a tiny mispositioned box instead
of a full-screen modal — the overlay just never visibly opened. The multi-part contract lives
nowhere in the types, so an invalid composition produces silently-broken output.

### I put two `Style`s in one bundle and it panicked at runtime

Rewriting `accordion()`, I built a bundle along the lines of `(Node, SelectGroup, Style, Style)`.
Bevy bundles don't dedupe, so it compiled and then panicked: *"duplicate components."* `Style`
being a bare component with no guard against this invites exactly the mistake.

### My `accordion()` rewrite silently dropped the background and swapped the shadow for a border

The accordion card needs `card_style()` (background + radius) **plus** a separate `card_shadow()`
`BoxShadow` — the elevation is deliberately a shadow, not a border, because a border on a rounded
box leaks at the corners. My rewrite dropped the background, replaced the shadow with a border,
and duplicated the `Style` — and still compiled. The correct structure lived only in the original
source, with no shared named building block, so the edit regressed the visuals quietly.

### My overlays snapped open at full opacity instead of animating in

In `advance_overlays` the `else` branch aimed Motion opacity → 1, and that branch covered *both*
the open state and the resting-closed state. So a closed overlay (`Display::None`) kept easing its
opacity up to 1.0 while hidden; on open it was already fully opaque and appeared instantly with no
enter animation. I fixed it by holding the pre-enter pose (opacity 0, `content.enter` transform)
while resting closed. The state machine had conflated "closed" with "open" for Motion.

### Driving an overlay open from a consumer system lagged a frame

The ui reactive systems run before a consumer's `Update` system by default, so when I drove an
overlay open from the gallery, `advance_overlays` only saw it the *next* frame. On a full-screen
scrim that one-frame lag was a multi-percent per-frame pixel diff. I fixed it two ways: a public
`ui::set_overlay_open(world, entity, open)` that walks the portal to the `Open` root and sets it
directly (no observer/command hop), and a public `UiReactive` `SystemSet` so a consumer can order
its mutation before the ui pass. Until that anchor existed, mutating ui state in `Update` silently
raced the reactive systems.

### I assumed a uniform way to set a component's value — there wasn't one

Wiring the gallery to drive each component, I kept writing against the wrong control surface
because they're a grab-bag: overlays open via `Pointer<Click>` → `OverlayAction` and re-parent
their content out of the trigger's subtree (so walking `ChildOf` from a trigger does *not* reach
the `Open` root — it leads to the host); select groups react to `Activate`; tooltips to
`Pointer<Over>` plus a hidden timer; sliders expose a public `SliderState`, but progress's
`ProgressFraction` **wasn't even re-exported** (I had to make it public). I'd expected one "set
this component's value/state" path and repeatedly reached for the wrong one.
