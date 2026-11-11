//! Trabecular-bone infill generator for FDM 3D printing.
//!
//! The pipeline voxelises a closed mesh, keeps a solid skin, fills the core
//! with a perforated-sheet lattice derived from Gaussian random fields, makes
//! that lattice self-supporting, and returns a mesh ready for a slicer.
//! See `docs/ALGORITHM.md` for the full description.

// Index loops are deliberate in the numeric kernels: several parallel
// buffers are addressed by the same voxel index.
#![allow(clippy::needless_range_loop)]

pub mod analyze;
pub mod app;
pub mod bridge;
pub mod edt;
pub mod field;
pub mod filters;
pub mod grid;
pub mod io;
pub mod isosurface;
pub mod mathx;
pub mod mesh;
pub mod noise;
pub mod params;
pub mod pipeline;
pub mod render;
pub mod schema;
pub mod sdf;
pub mod section;
#[cfg(feature = "cli")]
pub mod serve;
pub mod simplify;
pub mod support;
pub mod util;
pub mod vault;
pub mod viewer;
pub mod volume;
pub mod voxelize;
