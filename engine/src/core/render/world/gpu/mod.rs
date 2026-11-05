//! The draw path the world moves onto: what it draws lives on the GPU and is drawn indirectly,
//! queued straight into bevy's opaque, prepass and shadow phases with pipelines built on the mesh
//! pipeline's view layouts, so it is lit, shadowed, occluded and smoothed like every mesh.

pub mod draw;
