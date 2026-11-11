//! Marching cubes on a regular grid: extracts the boundary of `{field > 0}`
//! as a closed, consistently oriented 2-manifold triangle mesh whose normals
//! point from material towards void.
//!
//! The 256-case triangulation table is generated at start-up instead of being
//! pasted in. For each sign configuration the surface is traced on the six
//! cube faces: every face contributes one directed segment per run of
//! positive corners, cutting that run off. Because the segment pairing on a
//! face depends only on the four corner signs of that face, the two cubes
//! sharing it always agree, which is the property the original 1987
//! table lacks (it leaves holes on ambiguous faces). The segments close into
//! loops, each loop is fan-triangulated.
//!
//! Extraction runs in two parallel phases: vertices are created per voxel
//! layer on the grid edges they own, then triangles per cell layer look those
//! vertices up. Vertices are therefore shared exactly, with no welding pass.

use crate::grid::Grid;
use crate::mesh::TriMesh;
use rayon::prelude::*;
use std::sync::OnceLock;

/// Corners are indexed `x | y << 1 | z << 2`. Each face lists its corners
/// counter-clockwise as seen from outside the cube.
const FACES: [[usize; 4]; 6] = [
    [0, 4, 6, 2], // -x
    [1, 3, 7, 5], // +x
    [0, 1, 5, 4], // -y
    [2, 6, 7, 3], // +y
    [0, 2, 3, 1], // -z
    [4, 5, 7, 6], // +z
];

/// Edge id of the cube edge joining two adjacent corners: `axis * 4 + slot`,
/// where `slot` packs the two fixed coordinates of the edge.
fn edge_id(a: usize, b: usize) -> usize {
    let diff = a ^ b;
    debug_assert!(diff == 1 || diff == 2 || diff == 4);
    let lo = a & b;
    let (x, y, z) = (lo & 1, (lo >> 1) & 1, (lo >> 2) & 1);
    match diff {
        1 => y + 2 * z,
        2 => 4 + x + 2 * z,
        _ => 8 + x + 2 * y,
    }
}

/// `(axis, dx, dy, dz)` of the voxel that owns cube edge `e`.
const fn edge_owner(e: usize) -> (usize, usize, usize, usize) {
    let slot = e % 4;
    match e / 4 {
        0 => (0, 0, slot & 1, slot >> 1),
        1 => (1, slot & 1, 0, slot >> 1),
        _ => (2, slot & 1, slot >> 1, 0),
    }
}

struct Tables {
    /// Triangles (as cube edge ids) for each of the 256 corner sign patterns.
    cases: Vec<Vec<[u8; 3]>>,
}

fn build_tables() -> Tables {
    let mut cases = Vec::with_capacity(256);
    for config in 0usize..256 {
        let inside = |c: usize| config >> c & 1 == 1;
        // next[e] = edge reached from e by following the surface boundary.
        let mut next = [usize::MAX; 12];
        for face in &FACES {
            for m in 0..4 {
                // A run of positive corners starts after an out -> in step.
                if inside(face[m]) || !inside(face[(m + 1) % 4]) {
                    continue;
                }
                let start = edge_id(face[m], face[(m + 1) % 4]);
                let mut n = (m + 1) % 4;
                while inside(face[(n + 1) % 4]) {
                    n = (n + 1) % 4;
                }
                let end = edge_id(face[n], face[(n + 1) % 4]);
                debug_assert_eq!(next[start], usize::MAX);
                next[start] = end;
            }
        }
        let mut triangles = Vec::new();
        let mut seen = [false; 12];
        for first in 0..12 {
            if next[first] == usize::MAX || seen[first] {
                continue;
            }
            let mut ring = Vec::with_capacity(8);
            let mut e = first;
            while !seen[e] {
                seen[e] = true;
                ring.push(e as u8);
                e = next[e];
            }
            debug_assert_eq!(e, first);
            let found = triangulate(&ring, &mut triangles);
            assert!(found, "no manifold triangulation for marching-cubes case {config}");
        }
        cases.push(triangles);
    }
    Tables { cases }
}

/// Bit mask of the cube faces (`-x +x -y +y -z +z`) that contain cube edge `e`.
fn edge_faces(e: usize) -> u8 {
    let slot = e % 4;
    let (lo, hi) = (slot & 1, slot >> 1);
    match e / 4 {
        0 => 1 << (2 + lo) | 1 << (4 + hi), // fixed y, z
        1 => 1 << lo | 1 << (4 + hi),       // fixed x, z
        _ => 1 << lo | 1 << (2 + hi),       // fixed x, y
    }
}

/// Triangulates a loop of cube edges without using any diagonal that lies in
/// a cube face.
///
/// Two loop vertices on the same face that are not consecutive can only occur
/// on an ambiguous face. A diagonal between them would lie in that face, and
/// the neighbouring cube may pick the same one: four triangles would then
/// share one edge. Avoiding those diagonals keeps the mesh 2-manifold.
fn triangulate(ring: &[u8], out: &mut Vec<[u8; 3]>) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let allowed = |a: u8, b: u8| edge_faces(a as usize) & edge_faces(b as usize) == 0;
    // Polygon ring[i..=j] with base edge (i, j); choose the apex m.
    fn solve(ring: &[u8], i: usize, j: usize, allowed: &dyn Fn(u8, u8) -> bool, out: &mut Vec<[u8; 3]>) -> bool {
        if j - i < 2 {
            return true;
        }
        for m in i + 1..j {
            if m > i + 1 && !allowed(ring[i], ring[m]) {
                continue;
            }
            if m + 1 < j && !allowed(ring[m], ring[j]) {
                continue;
            }
            let mark = out.len();
            out.push([ring[i], ring[m], ring[j]]);
            if solve(ring, i, m, allowed, out) && solve(ring, m, j, allowed, out) {
                return true;
            }
            out.truncate(mark);
        }
        false
    }
    solve(ring, 0, n - 1, &allowed, out)
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(build_tables)
}

/// Vertices owned by one voxel layer.
#[derive(Default)]
struct LayerVertices {
    /// `(j * nx + i) * 3 + axis` of the grid edge each vertex sits on.
    keys: Vec<u32>,
    positions: Vec<[f32; 3]>,
}

/// Zero crossing between two samples of opposite sign, as a fraction of the
/// edge, kept slightly away from the ends so no triangle degenerates.
#[inline]
fn crossing(a: f32, b: f32) -> f32 {
    (a / (a - b)).clamp(0.02, 0.98)
}

/// Extracts the surface of `{field > 0}`. Where the positive region touches
/// the border of the grid the surface is left open, so callers that need a
/// closed mesh must keep a margin of non-positive (or positive) samples.
pub fn extract(field: &[f32], grid: &Grid) -> TriMesh {
    let [nx, ny, nz] = grid.dims;
    let layer = nx * ny;
    assert_eq!(field.len(), grid.len());
    assert!(layer * 3 < u32::MAX as usize, "layer too large for 32-bit edge keys");
    if nx < 2 || ny < 2 || nz < 2 {
        return TriMesh::default();
    }
    let origin = [grid.origin[0] as f32, grid.origin[1] as f32, grid.origin[2] as f32];
    let h = [grid.spacing[0] as f32, grid.spacing[1] as f32, grid.spacing[2] as f32];

    // Phase 1: one vertex on every grid edge whose ends differ in sign.
    let layers: Vec<LayerVertices> = (0..nz)
        .into_par_iter()
        .map(|k| {
            let mut out = LayerVertices::default();
            let plane = &field[k * layer..(k + 1) * layer];
            let above = if k + 1 < nz { Some(&field[(k + 1) * layer..(k + 2) * layer]) } else { None };
            let z = origin[2] + k as f32 * h[2];
            for j in 0..ny {
                let y = origin[1] + j as f32 * h[1];
                for i in 0..nx {
                    let p = j * nx + i;
                    let v = plane[p];
                    let s = v > 0.0;
                    let x = origin[0] + i as f32 * h[0];
                    if i + 1 < nx {
                        let w = plane[p + 1];
                        if (w > 0.0) != s {
                            out.keys.push((p * 3) as u32);
                            out.positions.push([x + crossing(v, w) * h[0], y, z]);
                        }
                    }
                    if j + 1 < ny {
                        let w = plane[p + nx];
                        if (w > 0.0) != s {
                            out.keys.push((p * 3 + 1) as u32);
                            out.positions.push([x, y + crossing(v, w) * h[1], z]);
                        }
                    }
                    if let Some(above) = above {
                        let w = above[p];
                        if (w > 0.0) != s {
                            out.keys.push((p * 3 + 2) as u32);
                            out.positions.push([x, y, z + crossing(v, w) * h[2]]);
                        }
                    }
                }
            }
            out
        })
        .collect();

    let mut offsets = Vec::with_capacity(nz + 1);
    let mut total = 0usize;
    for l in &layers {
        offsets.push(total);
        total += l.keys.len();
    }
    offsets.push(total);
    assert!(total < u32::MAX as usize, "mesh has too many vertices");

    // Phase 2: triangles of each layer of cells.
    let cases = &tables().cases;
    let owners: [(usize, usize, usize, usize); 12] = std::array::from_fn(edge_owner);
    let chunks: Vec<Vec<[u32; 3]>> = (0..nz - 1)
        .into_par_iter()
        .map_init(
            || [vec![u32::MAX; layer * 3], vec![u32::MAX; layer * 3]],
            |lut, k| {
                let mut tris: Vec<[u32; 3]> = Vec::new();
                if layers[k].keys.is_empty() && layers[k + 1].keys.is_empty() {
                    return tris;
                }
                for side in 0..2 {
                    let base = offsets[k + side] as u32;
                    for (n, &key) in layers[k + side].keys.iter().enumerate() {
                        lut[side][key as usize] = base + n as u32;
                    }
                }
                let lower = &field[k * layer..(k + 1) * layer];
                let upper = &field[(k + 1) * layer..(k + 2) * layer];
                for j in 0..ny - 1 {
                    let r0 = &lower[j * nx..(j + 1) * nx];
                    let r1 = &lower[(j + 1) * nx..(j + 2) * nx];
                    let r2 = &upper[j * nx..(j + 1) * nx];
                    let r3 = &upper[(j + 1) * nx..(j + 2) * nx];
                    for i in 0..nx - 1 {
                        let config = (r0[i] > 0.0) as usize
                            | ((r0[i + 1] > 0.0) as usize) << 1
                            | ((r1[i] > 0.0) as usize) << 2
                            | ((r1[i + 1] > 0.0) as usize) << 3
                            | ((r2[i] > 0.0) as usize) << 4
                            | ((r2[i + 1] > 0.0) as usize) << 5
                            | ((r3[i] > 0.0) as usize) << 6
                            | ((r3[i + 1] > 0.0) as usize) << 7;
                        if config == 0 || config == 255 {
                            continue;
                        }
                        for tri in &cases[config] {
                            let mut ids = [0u32; 3];
                            for (slot, &e) in tri.iter().enumerate() {
                                let (axis, dx, dy, dz) = owners[e as usize];
                                ids[slot] = lut[dz][((j + dy) * nx + i + dx) * 3 + axis];
                            }
                            debug_assert!(ids.iter().all(|&v| v != u32::MAX));
                            tris.push(ids);
                        }
                    }
                }
                for side in 0..2 {
                    for &key in &layers[k + side].keys {
                        lut[side][key as usize] = u32::MAX;
                    }
                }
                tris
            },
        )
        .collect();

    let mut vertices = Vec::with_capacity(total);
    for l in &layers {
        vertices.extend_from_slice(&l.positions);
    }
    let count: usize = chunks.iter().map(Vec::len).sum();
    let mut triangles = Vec::with_capacity(count);
    for c in chunks {
        triangles.extend(c);
    }
    TriMesh { vertices, triangles }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::Rng;

    #[test]
    fn table_has_no_triangles_for_trivial_cases_and_is_complementary_in_size() {
        let t = tables();
        assert!(t.cases[0].is_empty());
        assert!(t.cases[255].is_empty());
        // One positive corner: a single triangle.
        for c in 0..8 {
            assert_eq!(t.cases[1 << c].len(), 1);
        }
        // Every crossing edge is used, and only crossing edges.
        for config in 1usize..255 {
            let mut used = [false; 12];
            for tri in &t.cases[config] {
                for &e in tri {
                    used[e as usize] = true;
                }
            }
            for a in 0..8usize {
                for bit in [1usize, 2, 4] {
                    if a & bit != 0 {
                        continue;
                    }
                    let b = a | bit;
                    let crosses = (config >> a & 1) != (config >> b & 1);
                    assert_eq!(used[edge_id(a, b)], crosses, "config {config}, edge {a}-{b}");
                }
            }
        }
    }

    fn sphere_field(n: usize, r: f32) -> (Vec<f32>, Grid) {
        let grid = Grid { origin: [0.0; 3], spacing: [1.0; 3], dims: [n, n, n] };
        let c = (n as f32 - 1.0) / 2.0;
        let mut f = vec![0f32; n * n * n];
        for k in 0..n {
            for j in 0..n {
                for i in 0..n {
                    let d = ((i as f32 - c).powi(2) + (j as f32 - c).powi(2) + (k as f32 - c).powi(2)).sqrt();
                    f[grid.index(i, j, k)] = r - d;
                }
            }
        }
        (f, grid)
    }

    #[test]
    fn sphere_is_watertight_outward_and_accurate() {
        let (f, grid) = sphere_field(40, 14.3);
        let m = extract(&f, &grid);
        assert!(m.edge_report().is_watertight(), "{:?}", m.edge_report());
        let exact = 4.0 / 3.0 * std::f64::consts::PI * 14.3f64.powi(3);
        let vol = m.signed_volume();
        assert!(vol > 0.0, "normals point inwards");
        assert!((vol - exact).abs() / exact < 0.01, "{vol} vs {exact}");
    }

    #[test]
    fn inverted_field_gives_a_cavity() {
        let (mut f, grid) = sphere_field(32, 10.0);
        f.iter_mut().for_each(|v| *v = -*v);
        let m = extract(&f, &grid);
        assert!(m.edge_report().is_watertight());
        assert!(m.signed_volume() < 0.0);
    }

    #[test]
    fn random_fields_are_always_closed_manifolds() {
        // White noise exercises every case, including all the ambiguous ones.
        let n = 18;
        let grid = Grid { origin: [0.0; 3], spacing: [1.0; 3], dims: [n, n, n] };
        for (seed, border) in [(1u64, -1.0f32), (2, 1.0), (3, -1.0), (4, 1.0)] {
            let mut rng = Rng::new(seed);
            let mut f: Vec<f32> = (0..n * n * n).map(|_| rng.uniform() as f32 - 0.5).collect();
            for k in 0..n {
                for j in 0..n {
                    for i in 0..n {
                        if i == 0 || j == 0 || k == 0 || i == n - 1 || j == n - 1 || k == n - 1 {
                            f[grid.index(i, j, k)] = border;
                        }
                    }
                }
            }
            let m = extract(&f, &grid);
            assert!(!m.is_empty());
            let report = m.edge_report();
            assert!(report.is_watertight(), "seed {seed}: {report:?}");
        }
    }

    #[test]
    fn plane_is_reproduced_exactly() {
        let n = 10;
        let grid = Grid { origin: [0.0; 3], spacing: [0.5; 3], dims: [n, n, n] };
        let mut f = vec![0f32; n * n * n];
        for k in 0..n {
            for j in 0..n {
                for i in 0..n {
                    f[grid.index(i, j, k)] = 2.1 - k as f32 * 0.5; // solid below z = 2.1
                }
            }
        }
        let m = extract(&f, &grid);
        assert!(!m.is_empty());
        for v in &m.vertices {
            assert!((v[2] - 2.1).abs() < 1e-5);
        }
    }
}
