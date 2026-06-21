# `ui` crate — code mistakes & design defects

Genuine bugs and design flaws found **in the `ui` crate code** during the `bevy_view` → `bevy_ui`
refactor and the pixel-parity work. Each is a place the crate's API or implementation let a wrong
thing compile, run, or render silently. (Process, gallery-port, and verification-method notes are
deliberately excluded.) Status: `fixed` / `open`.

---

1. **`UiPlugin` clobbers a consumer-configured `Theme`.** `build()` did
   `app.insert_resource(theme::default_theme())` unconditionally, overwriting a `Theme` the app set
   *before* adding the plugin. Result: a consumer that set the light theme silently ran on the dark
   default; only large theme-sensitive surfaces revealed it. *Fixed:* `if !contains_resource::<Theme>`.
   **Defect:** theme-as-a-single-global-`Resource` + plugin-inserts-default is order-sensitive; the
   plugin must not override app config, and a deliberate "theme provider" story would be safer.

2. **Inconsistent "who owns the `Node`" across components.** `tooltip()` returned no `Node` (the
   trigger node carries it so floating content anchors to the trigger), but `popover()` — an
   identically-anchored overlay — returned `(Node, Open, Dismissable)` *with* a `Node`. Composing the
   popover with a positioning `Node` panicked: *"Bundle … has duplicate components"*. *Fixed:* popover
   dropped its `Node`. **Defect:** there's no rule for whether a component free-fn includes its own
   `Node`; getting it wrong compiles and panics at runtime.

3. **Node-less component roots silently break the UI hierarchy (`B0004`).** Because `tooltip()` /
   `popover()` carry no `Node` (see #2), nesting them between Node entities (e.g. `Node` → marker
   (no Node) → `popover()` (no Node) → trigger `Node`) makes Bevy log `warning[B0004]: Node entity has
   a parent without Node` and **the entire subtree fails to lay out — nothing under it renders**. No
   panic; the view just comes up empty. **Defect:** a component that omits its `Node` is a hierarchy
   landmine — any non-Node entity between Nodes detaches everything below it, with only a warning.
   Component free-fns should always include a `Node` (or use a marker that can't be placed without one).

4. **Overlay multi-part composition is implicit and unenforced.** A working dialog requires
   `dialog()` + `dialog_modal()` (the Portal that re-parents content to a full-screen overlay host) +
   `dialog_scrim()` + `dialog_content()`. Omitting `dialog_modal()` compiles, runs, doesn't panic, and
   renders a tiny mispositioned box instead of a full-screen modal — the overlay never visibly opens.
   **Defect:** the multi-part contract is encoded nowhere; an invalid composition silently produces
   broken output. It should be enforced (a single `dialog(...)` that owns its scrim/content, or a
   required-children check).

5. **`Style` is a duplicate-prone plain component.** Two `Style`s in one bundle (e.g.
   `(Node, SelectGroup, Style, Style)`) compiles but panics at runtime ("duplicate components"); a
   rewrite of `accordion()` did exactly this. Bevy bundles don't dedupe and nothing guides authors
   toward one merged `Style`. **Defect:** the styling primitive being a bare component invites this; a
   builder that guarantees one `Style` per entity (or a compile-time guard) would prevent it.

6. **Per-component composition is bespoke and fragile.** `accordion()`'s card needs `card_style()`
   (background + radius) **plus** a separate `card_shadow()` `BoxShadow` (elevation, deliberately not a
   border because borders on rounded boxes leak at the corners). A rewrite silently dropped the
   background, swapped the shadow for a border, and duplicated the `Style` — and still compiled.
   **Defect:** lots of per-component hand-rolled composition with no shared named building blocks → any
   edit risks subtle visual regressions, and the "correct" structure lives only in the original source.

7. **Overlays snapped open at full opacity instead of animating their enter.** `advance_overlays`
   aimed Motion opacity→1 in its `else` branch, which covered BOTH the open state AND the
   resting-closed state. So while an overlay sat closed (`Display::None`), its Motion still eased
   opacity up to 1.0; on open it was already at full opacity and appeared instantly with no enter
   animation. *Fixed:* the resting-closed case now holds the pre-enter pose (opacity 0, `content.enter`
   transform). **Defect:** the open/closed/closing state machine conflated "closed" with "open" for
   Motion — a hidden element should not advance its enter animation.

8. **Overlay open lags a frame, and there's no public ordering anchor.** Driving an overlay open from
   a consumer system in `Update` was seen by `advance_overlays` on the *next* frame, because the ui
   reactive systems run before a consumer's input system by default — a 1-frame lag that, on a
   full-screen scrim, is a multi-percent per-frame diff. *Fixed two ways:* (a) added public
   `ui::set_overlay_open(world, entity, open)` that walks the portal to the `Open` root and sets it
   directly (no observer/command hop), and (b) the consumer must order its system before the ui
   reactive pass. **Defect:** the ui reactive systems expose no public `SystemSet`, so a consumer that
   mutates ui state in `Update` silently races them; there should be a documented ordering anchor.

9. **No uniform, public way to drive a component to a state/value.** Controlling components externally
   requires a grab-bag of private, asymmetric mechanisms: overlays open via `Pointer<Click>` →
   `OverlayAction` and re-parent their content out of the trigger's subtree, so walking `ChildOf` from
   a trigger/close-button does NOT reach the `Open` root (it leads to the host) — the only portal-aware
   path was the crate's own observer until `set_overlay_open` (see #8) was added; select groups react
   to `Activate`; tooltips to `Pointer<Over>` + a hidden timer; sliders to `SliderState` (public) but
   progress to `ProgressFraction` (which was **not** re-exported — fixed). **Defect:** components are
   externally controllable only through partly-private, inconsistent paths; a uniform "set this
   component's value/state" surface (or at least public state markers) is missing.

10. **Retained selection settles one frame late.** `init_selection` / `apply_start_checked` /
    `inherit_checked` stamp `Checked` via `Added` detection + `Commands`, i.e. the frame *after* spawn,
    whereas the original set selection during the first render. This is a built-in 1-frame lag at view
    start for tabs/radio/accordion/checkbox/switch. *Open.* **Defect:** multi-system, deferred-command
    state propagation has latency the single-pass render didn't; it shows at view boundaries.
