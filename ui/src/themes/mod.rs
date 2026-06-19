//! The concrete themes — each fills the [`Theme`](crate::theme::Theme) contract by mapping every
//! semantic color slot to a [`palette`](crate::tokens::palette) color.
//!  Supply one to a subtree with [`provide_theme`](crate::theme::provide_theme).

pub mod dark;
pub mod light;
