//! Cross-cutting mechanism shared by the components: the [`recipe`] styling engine, the [`theme`]
//! variable system it resolves against, the [`motion`] tween/animation engine, the [`interaction`]
//! pointer-state tracking that drives hover/press styling, and the [`controlled`] value-sharing
//! plumbing.

pub mod collapse;
pub mod controlled;
pub mod interaction;
pub mod motion;
pub mod popper;
pub mod recipe;
pub mod theme;
