//! Regular voxel grid description.
//!
//! Scalar fields are stored as flat `Vec`s in **X-fastest, Z-slowest** order,
//! so one Z layer (one printed layer of the lattice) is a contiguous slice of
//! `nx * ny` values. Every layer-wise algorithm (field evaluation, the
//! overhang filter, meshing) parallelises over those slices.

/// Voxel flags packed in one byte per voxel.
pub mod flag {
    /// Voxel centre is inside the input model.
    pub const INSIDE: u8 = 1 << 0;
    /// Inside the model, or in front of a face the user asked to open.
    pub const EXT: u8 = 1 << 1;
    /// Solid skin ("cortical bone"): never modified by the lattice passes,
    /// except along opened faces ([`RIM`]).
    pub const SHELL: u8 = 1 << 2;
    /// Skin along the edge of an opened face. The skin that held it up may
    /// have been opened away, so it follows the printing rules like the
    /// lattice instead of being taken as printable.
    pub const RIM: u8 = 1 << 3;
    /// Scratch bit used by flood fills.
    pub const VISITED: u8 = 1 << 7;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    /// World position (mm) of the centre of voxel `(0, 0, 0)`.
    pub origin: [f64; 3],
    /// Voxel size (mm) along each axis.
    pub spacing: [f64; 3],
    /// Number of voxels along each axis.
    pub dims: [usize; 3],
}

impl Grid {
    /// Isotropic grid enclosing `lo..hi` with `margin` millimetres of empty
    /// space on every side.
    pub fn enclosing(lo: [f64; 3], hi: [f64; 3], voxel: f64, margin: f64) -> Self {
        let mut origin = [0.0; 3];
        let mut dims = [0usize; 3];
        for a in 0..3 {
            origin[a] = lo[a] - margin;
            let extent = (hi[a] + margin) - origin[a];
            dims[a] = (extent / voxel).floor() as usize + 2;
        }
        Self { origin, spacing: [voxel; 3], dims }
    }

    #[inline]
    pub fn nx(&self) -> usize {
        self.dims[0]
    }
    #[inline]
    pub fn ny(&self) -> usize {
        self.dims[1]
    }
    #[inline]
    pub fn nz(&self) -> usize {
        self.dims[2]
    }

    /// Total number of voxels.
    #[inline]
    pub fn len(&self) -> usize {
        self.dims[0] * self.dims[1] * self.dims[2]
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of voxels in one Z layer.
    #[inline]
    pub fn layer_len(&self) -> usize {
        self.dims[0] * self.dims[1]
    }

    #[inline]
    pub fn index(&self, i: usize, j: usize, k: usize) -> usize {
        (k * self.dims[1] + j) * self.dims[0] + i
    }

    /// Voxel size of an isotropic grid.
    #[inline]
    pub fn voxel(&self) -> f64 {
        debug_assert!(self.spacing[0] == self.spacing[1] && self.spacing[1] == self.spacing[2]);
        self.spacing[0]
    }

    #[inline]
    pub fn x(&self, i: usize) -> f64 {
        self.origin[0] + i as f64 * self.spacing[0]
    }
    #[inline]
    pub fn y(&self, j: usize) -> f64 {
        self.origin[1] + j as f64 * self.spacing[1]
    }
    #[inline]
    pub fn z(&self, k: usize) -> f64 {
        self.origin[2] + k as f64 * self.spacing[2]
    }

    /// Coordinates of all voxel centres along one axis, as `f32`.
    pub fn axis_coords(&self, axis: usize) -> Vec<f32> {
        (0..self.dims[axis]).map(|i| (self.origin[axis] + i as f64 * self.spacing[axis]) as f32).collect()
    }

    /// Index of the voxel whose centre is nearest to `p`, if inside the grid.
    pub fn nearest(&self, p: [f64; 3]) -> Option<[usize; 3]> {
        let mut out = [0usize; 3];
        for a in 0..3 {
            let f = ((p[a] - self.origin[a]) / self.spacing[a]).round();
            if f < 0.0 || f >= self.dims[a] as f64 {
                return None;
            }
            out[a] = f as usize;
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enclosing_covers_bounds_with_margin() {
        let g = Grid::enclosing([0.0, 0.0, 0.0], [10.0, 5.0, 2.0], 0.3, 1.6);
        for a in 0..3 {
            let last = g.origin[a] + (g.dims[a] - 1) as f64 * g.spacing[a];
            assert!(g.origin[a] <= -1.6 + 1e-12);
            assert!(last >= [10.0, 5.0, 2.0][a] + 1.6);
        }
        assert_eq!(g.index(1, 2, 3), (3 * g.ny() + 2) * g.nx() + 1);
    }
}
