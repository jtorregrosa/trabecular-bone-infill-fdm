//! Mesh → voxel occupancy by scanline ray casting along X.
//!
//! Every `(j, k)` grid row is a ray parallel to X. Each triangle is projected
//! on the YZ plane and rasterised with half-open rules, so a ray that passes
//! exactly through a shared edge or vertex is counted once, never twice or
//! zero times. Crossings carry the sign of the facet normal and rows are
//! filled with the non-zero winding rule, which also handles meshes with
//! cavities (inverted inner shells) and overlapping closed shells.

use crate::grid::Grid;
use crate::mesh::TriMesh;
use rayon::prelude::*;

#[derive(Clone, Copy)]
struct Hit {
    row: u32,
    x: f32,
    sign: i8,
}

/// Returns one byte per voxel: 1 if the voxel centre is inside the mesh.
pub fn voxelize(mesh: &TriMesh, grid: &Grid) -> Vec<u8> {
    let [nx, ny, nz] = grid.dims;
    assert!(ny * nz <= u32::MAX as usize, "grid has too many rows");
    let mut out = vec![0u8; grid.len()];
    if mesh.is_empty() || out.is_empty() {
        return out;
    }

    let mut hits: Vec<Hit> = mesh
        .triangles
        .par_iter()
        .fold(Vec::new, |mut acc, t| {
            scan_triangle(mesh, t, grid, &mut acc);
            acc
        })
        .reduce(Vec::new, |mut a, mut b| {
            if a.len() < b.len() {
                std::mem::swap(&mut a, &mut b);
            }
            a.append(&mut b);
            a
        });
    hits.par_sort_unstable_by(|a, b| a.row.cmp(&b.row).then(a.x.total_cmp(&b.x)));

    // CSR offsets: hits of row r are hits[start[r]..start[r + 1]].
    let rows = ny * nz;
    let mut start = vec![0u32; rows + 1];
    for h in &hits {
        start[h.row as usize + 1] += 1;
    }
    for r in 0..rows {
        start[r + 1] += start[r];
    }

    let ox = grid.origin[0];
    let inv_dx = 1.0 / grid.spacing[0];
    out.par_chunks_mut(nx).enumerate().for_each(|(row, line)| {
        let row_hits = &hits[start[row] as usize..start[row + 1] as usize];
        let mut winding = 0i32;
        for pair in row_hits.windows(2) {
            winding += pair[0].sign as i32;
            if winding == 0 {
                continue;
            }
            // Voxel centres with x in [x_a, x_b).
            let a = ((pair[0].x as f64 - ox) * inv_dx).ceil().max(0.0) as usize;
            let b = (((pair[1].x as f64 - ox) * inv_dx).ceil().max(0.0) as usize).min(nx);
            if a < b {
                line[a..b].fill(1);
            }
        }
    });
    out
}

fn scan_triangle(mesh: &TriMesh, t: &[u32; 3], grid: &Grid, acc: &mut Vec<Hit>) {
    let p = [
        crate::mesh::to_f64(mesh.vertices[t[0] as usize]),
        crate::mesh::to_f64(mesh.vertices[t[1] as usize]),
        crate::mesh::to_f64(mesh.vertices[t[2] as usize]),
    ];
    // X component of the facet normal = signed area of the YZ projection.
    let nx_component = (p[1][1] - p[0][1]) * (p[2][2] - p[0][2]) - (p[1][2] - p[0][2]) * (p[2][1] - p[0][1]);
    if nx_component == 0.0 {
        return;
    }
    // A ray travelling towards +X enters the solid through facets facing -X.
    let sign: i8 = if nx_component < 0.0 { 1 } else { -1 };

    let [_, ny, nz] = grid.dims;
    let (oy, oz) = (grid.origin[1], grid.origin[2]);
    let (dy, dz) = (grid.spacing[1], grid.spacing[2]);
    let zmin = p[0][2].min(p[1][2]).min(p[2][2]);
    let zmax = p[0][2].max(p[1][2]).max(p[2][2]);
    let k0 = ((zmin - oz) / dz).ceil().max(0.0) as usize;
    let k1 = (((zmax - oz) / dz).ceil().max(0.0) as usize).min(nz);

    for k in k0..k1 {
        let zk = oz + k as f64 * dz;
        // Intersect the three edges with the line z = zk. Edges are half-open
        // in z ([lower, upper)) and always evaluated from their lower endpoint
        // so both triangles sharing an edge compute the same crossing point.
        let mut pts = [(0.0f64, 0.0f64); 2];
        let mut n = 0usize;
        for e in 0..3 {
            let (mut a, mut b) = (p[e], p[(e + 1) % 3]);
            if a[2] > b[2] {
                std::mem::swap(&mut a, &mut b);
            }
            if a[2] <= zk && zk < b[2] {
                let s = (zk - a[2]) / (b[2] - a[2]);
                if n < 2 {
                    pts[n] = (a[1] + s * (b[1] - a[1]), a[0] + s * (b[0] - a[0]));
                }
                n += 1;
            }
        }
        if n != 2 {
            continue;
        }
        let ((y0, x0), (y1, x1)) = if pts[0].0 <= pts[1].0 { (pts[0], pts[1]) } else { (pts[1], pts[0]) };
        if y1 <= y0 {
            continue;
        }
        // Rows with y in [y0, y1).
        let j0 = ((y0 - oy) / dy).ceil().max(0.0) as usize;
        let j1 = (((y1 - oy) / dy).ceil().max(0.0) as usize).min(ny);
        for j in j0..j1 {
            let yj = oy + j as f64 * dy;
            let s = (yj - y0) / (y1 - y0);
            acc.push(Hit { row: (k * ny + j) as u32, x: (x0 + s * (x1 - x0)) as f32, sign });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::box_mesh;

    fn count(v: &[u8]) -> usize {
        v.iter().filter(|&&b| b != 0).count()
    }

    #[test]
    fn box_volume_is_exact_on_aligned_grid() {
        // Faces coincide with voxel centres on purpose: half-open rules must
        // count each boundary plane exactly once.
        let m = box_mesh([0.0; 3], [2.0, 3.0, 4.0]);
        let g = Grid { origin: [-1.0; 3], spacing: [0.5; 3], dims: [10, 12, 14] };
        let v = voxelize(&m, &g);
        // Centres in [0, 2) x [0, 3) x [0, 4) at 0.5 mm pitch: 4 * 6 * 8.
        assert_eq!(count(&v), 4 * 6 * 8);
    }

    #[test]
    fn cavity_is_empty() {
        let mut m = box_mesh([0.0; 3], [4.0; 3]);
        let mut hole = box_mesh([1.0; 3], [3.0; 3]);
        hole.flip();
        m.append(&hole);
        let g = Grid { origin: [-0.75; 3], spacing: [0.5; 3], dims: [12, 12, 12] };
        let v = voxelize(&m, &g);
        // Centres at -0.75 + 0.5 i: 8 per axis inside (0,4), 4 per axis inside (1,3).
        assert_eq!(count(&v), 8 * 8 * 8 - 4 * 4 * 4);
        let centre = g.index(6, 6, 6);
        assert_eq!(v[centre], 0);
    }

    #[test]
    fn rotated_tetrahedron_volume_converges() {
        let vertices = vec![[0.1, 0.2, 0.3], [5.3, 0.7, 0.4], [1.1, 4.9, 0.9], [2.0, 1.5, 6.1]];
        let mut m = TriMesh { vertices, triangles: vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]] };
        if m.signed_volume() < 0.0 {
            m.flip();
        }
        assert!(m.edge_report().is_watertight());
        let g = Grid::enclosing([0.0; 3], [5.5, 5.0, 6.2], 0.05, 0.2);
        let v = voxelize(&m, &g);
        let vol = count(&v) as f64 * 0.05f64.powi(3);
        let exact = m.signed_volume();
        assert!((vol - exact).abs() / exact < 0.01, "voxel {vol} vs exact {exact}");
    }
}
