//! Mesh simplification by quadric-error edge collapse (Garland & Heckbert).
//!
//! Marching cubes produces triangles the size of a voxel everywhere, far more
//! than the smooth lattice surfaces need. Edges are collapsed, cheapest
//! first, while the merged vertex stays within the tolerance of the planes
//! of *all* the original triangles it has come to stand for. (The quadric
//! error is the sum of the squared distances to those planes, so keeping it
//! under `tolerance^2` bounds every one of them.) Flat areas therefore
//! collapse almost entirely, while corners and creases stay put.
//!
//! A collapse is refused when it would break the surface: the *link
//! condition* keeps the mesh a closed 2-manifold, and a normal test rejects
//! triangles that would flip over.
//!
//! Large meshes are cut into runs of consecutive triangles (marching cubes
//! emits them layer by layer, so a run is a slab), simplified in parallel,
//! and stitched back. Vertices shared between runs are frozen, which leaves a
//! thin band of original triangles at each seam.

use crate::mesh::TriMesh;
use crate::util::FxHashMap;
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Triangles per parallel run.
const RUN_TRIANGLES: usize = 150_000;

type Quadric = [f64; 10]; // a00 a01 a02 a11 a12 a22 b0 b1 b2 c

#[inline]
fn add(q: &mut Quadric, o: &Quadric) {
    for i in 0..10 {
        q[i] += o[i];
    }
}

#[inline]
fn error(q: &Quadric, p: [f64; 3]) -> f64 {
    let [x, y, z] = p;
    q[0] * x * x
        + 2.0 * q[1] * x * y
        + 2.0 * q[2] * x * z
        + q[3] * y * y
        + 2.0 * q[4] * y * z
        + q[5] * z * z
        + 2.0 * (q[6] * x + q[7] * y + q[8] * z)
        + q[9]
}

/// Point minimising the quadric, if the system is well conditioned.
fn optimum(q: &Quadric) -> Option<[f64; 3]> {
    let (a, b, c, d, e, f) = (q[0], q[1], q[2], q[3], q[4], q[5]);
    let det = a * (d * f - e * e) - b * (b * f - e * c) + c * (b * e - d * c);
    let scale = (a + d + f).abs().max(1e-30);
    if det.abs() < 1e-6 * scale * scale * scale {
        return None;
    }
    let (r0, r1, r2) = (-q[6], -q[7], -q[8]);
    let x = (r0 * (d * f - e * e) - b * (r1 * f - e * r2) + c * (r1 * e - d * r2)) / det;
    let y = (a * (r1 * f - e * r2) - r0 * (b * f - e * c) + c * (b * r2 - r1 * c)) / det;
    let z = (a * (d * r2 - r1 * e) - b * (b * r2 - r1 * c) + r0 * (b * e - d * c)) / det;
    Some([x, y, z])
}

#[inline]
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
#[inline]
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

struct Candidate {
    cost: f64,
    a: u32,
    b: u32,
    stamp_a: u32,
    stamp_b: u32,
}
impl PartialEq for Candidate {
    fn eq(&self, o: &Self) -> bool {
        self.cost == o.cost
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Candidate {
    fn cmp(&self, o: &Self) -> Ordering {
        // Min-heap on cost.
        o.cost.total_cmp(&self.cost)
    }
}

struct Simplifier {
    pos: Vec<[f64; 3]>,
    quadric: Vec<Quadric>,
    stamp: Vec<u32>,
    locked: Vec<bool>,
    tris: Vec<[u32; 3]>,
    alive: Vec<bool>,
    /// Triangles around each vertex (may contain dead ones; filtered on use).
    around: Vec<Vec<u32>>,
    limit: f64,
}

impl Simplifier {
    fn new(vertices: &[[f32; 3]], triangles: &[[u32; 3]], locked: Vec<bool>, tolerance: f64) -> Self {
        let pos: Vec<[f64; 3]> = vertices.iter().map(|v| [v[0] as f64, v[1] as f64, v[2] as f64]).collect();
        let mut quadric = vec![[0.0; 10]; pos.len()];
        let mut around: Vec<Vec<u32>> = vec![Vec::new(); pos.len()];
        for (t, tri) in triangles.iter().enumerate() {
            let [a, b, c] = tri.map(|v| pos[v as usize]);
            let n = cross(sub(b, a), sub(c, a));
            let len = dot(n, n).sqrt();
            if len > 0.0 {
                let n = [n[0] / len, n[1] / len, n[2] / len];
                let d = -dot(n, a);
                let q: Quadric = [
                    n[0] * n[0],
                    n[0] * n[1],
                    n[0] * n[2],
                    n[1] * n[1],
                    n[1] * n[2],
                    n[2] * n[2],
                    n[0] * d,
                    n[1] * d,
                    n[2] * d,
                    d * d,
                ];
                for &v in tri {
                    add(&mut quadric[v as usize], &q);
                }
            }
            for &v in tri {
                around[v as usize].push(t as u32);
            }
        }
        Self {
            stamp: vec![0; pos.len()],
            pos,
            quadric,
            locked,
            tris: triangles.to_vec(),
            alive: vec![true; triangles.len()],
            around,
            limit: tolerance * tolerance,
        }
    }

    /// Cheapest admissible way to merge `a` and `b`: summed squared distance
    /// to the planes both stand for, and the merged position.
    fn evaluate(&self, a: u32, b: u32) -> Option<(f64, [f64; 3])> {
        let (la, lb) = (self.locked[a as usize], self.locked[b as usize]);
        if la && lb {
            return None;
        }
        let mut q = self.quadric[a as usize];
        add(&mut q, &self.quadric[b as usize]);
        let (pa, pb) = (self.pos[a as usize], self.pos[b as usize]);
        let mut best: Option<(f64, [f64; 3])> = None;
        let mut consider = |p: [f64; 3]| {
            let e = error(&q, p).max(0.0);
            if best.is_none_or(|(be, _)| e < be) {
                best = Some((e, p));
            }
        };
        if la {
            consider(pa);
        } else if lb {
            consider(pb);
        } else {
            if let Some(p) = optimum(&q) {
                // Keep the optimum near the edge: on flat or straight parts
                // the minimiser can sit far away.
                let mid = [0.5 * (pa[0] + pb[0]), 0.5 * (pa[1] + pb[1]), 0.5 * (pa[2] + pb[2])];
                let reach = dot(sub(pa, pb), sub(pa, pb));
                if dot(sub(p, mid), sub(p, mid)) <= reach {
                    consider(p);
                }
            }
            consider(pa);
            consider(pb);
            consider([0.5 * (pa[0] + pb[0]), 0.5 * (pa[1] + pb[1]), 0.5 * (pa[2] + pb[2])]);
        }
        best.filter(|(e, _)| *e <= self.limit)
    }

    fn neighbours(&self, v: u32, out: &mut Vec<u32>) {
        out.clear();
        for &t in &self.around[v as usize] {
            if self.alive[t as usize] {
                for &w in &self.tris[t as usize] {
                    if w != v && !out.contains(&w) {
                        out.push(w);
                    }
                }
            }
        }
    }

    /// Would moving `v` to `p` flip or squash a triangle that survives the
    /// collapse of edge `v`–`other`?
    fn flips(&self, v: u32, other: u32, p: [f64; 3]) -> bool {
        for &t in &self.around[v as usize] {
            if !self.alive[t as usize] {
                continue;
            }
            let tri = self.tris[t as usize];
            if tri.contains(&other) {
                continue; // disappears with the edge
            }
            let before = tri.map(|w| self.pos[w as usize]);
            let after = tri.map(|w| if w == v { p } else { self.pos[w as usize] });
            let n0 = cross(sub(before[1], before[0]), sub(before[2], before[0]));
            let n1 = cross(sub(after[1], after[0]), sub(after[2], after[0]));
            let (l0, l1) = (dot(n0, n0).sqrt(), dot(n1, n1).sqrt());
            if l1 <= 1e-12 || dot(n0, n1) < 0.2 * l0 * l1 {
                return true;
            }
        }
        false
    }

    fn run(&mut self) {
        let mut heap = BinaryHeap::new();
        let mut edges: Vec<u64> = Vec::with_capacity(self.tris.len() * 3);
        for tri in &self.tris {
            for e in 0..3 {
                let (a, b) = (tri[e], tri[(e + 1) % 3]);
                if a < b {
                    edges.push((a as u64) << 32 | b as u64);
                }
            }
        }
        edges.sort_unstable();
        edges.dedup();
        for e in edges {
            let (a, b) = ((e >> 32) as u32, e as u32);
            if let Some((cost, _)) = self.evaluate(a, b) {
                heap.push(Candidate { cost, a, b, stamp_a: 0, stamp_b: 0 });
            }
        }

        let (mut na, mut nb) = (Vec::new(), Vec::new());
        while let Some(c) = heap.pop() {
            let (a, b) = (c.a, c.b);
            if self.stamp[a as usize] != c.stamp_a || self.stamp[b as usize] != c.stamp_b {
                continue; // stale: one end has changed since
            }
            let Some((_, p)) = self.evaluate(a, b) else { continue };
            // `keep` survives and moves to `p`; a locked end must be the one.
            let (keep, gone) = if self.locked[b as usize] { (b, a) } else { (a, b) };

            // Link condition: the ends share exactly the two vertices opposite
            // the edge. Anything else would pinch the surface.
            self.neighbours(keep, &mut na);
            self.neighbours(gone, &mut nb);
            if na.iter().filter(|v| nb.contains(v)).count() != 2 {
                continue;
            }
            // Merging into a seam vertex creates edges from it to the
            // neighbours of the vertex that goes. If one of those neighbours
            // is itself on the seam, the two may already be joined by an edge
            // in the adjacent run, which this run cannot see.
            if self.locked[keep as usize]
                && nb.iter().any(|&v| v != keep && self.locked[v as usize] && !na.contains(&v))
            {
                continue;
            }
            if self.flips(keep, gone, p) || self.flips(gone, keep, p) {
                continue;
            }

            let moved = std::mem::take(&mut self.around[gone as usize]);
            for t in moved {
                if !self.alive[t as usize] {
                    continue;
                }
                let tri = &mut self.tris[t as usize];
                if tri.contains(&keep) {
                    self.alive[t as usize] = false;
                } else {
                    for w in tri.iter_mut() {
                        if *w == gone {
                            *w = keep;
                        }
                    }
                    self.around[keep as usize].push(t);
                }
            }
            let alive = &self.alive;
            self.around[keep as usize].retain(|&t| alive[t as usize]);
            self.pos[keep as usize] = p;
            let q = self.quadric[gone as usize];
            add(&mut self.quadric[keep as usize], &q);
            self.stamp[keep as usize] += 1;
            self.stamp[gone as usize] = u32::MAX;

            self.neighbours(keep, &mut na);
            for &n in &na {
                if let Some((cost, _)) = self.evaluate(keep, n) {
                    heap.push(Candidate {
                        cost,
                        a: keep,
                        b: n,
                        stamp_a: self.stamp[keep as usize],
                        stamp_b: self.stamp[n as usize],
                    });
                }
            }
        }
    }
}

/// Simplifies one piece. Vertices flagged in `locked` neither move nor
/// disappear. Returns, per surviving triangle, its vertex ids (still indices
/// into the input), plus the final position of every vertex.
fn simplify_piece(
    vertices: &[[f32; 3]],
    triangles: &[[u32; 3]],
    locked: Vec<bool>,
    tolerance: f64,
) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let mut s = Simplifier::new(vertices, triangles, locked, tolerance);
    s.run();
    let positions = s.pos.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
    let tris = s.tris.iter().zip(&s.alive).filter(|(_, alive)| **alive).map(|(t, _)| *t).collect();
    (positions, tris)
}

/// Simplifies a closed mesh within `tolerance` millimetres.
pub fn simplify(mesh: &TriMesh, tolerance: f32) -> TriMesh {
    simplify_in_runs(mesh, tolerance, RUN_TRIANGLES)
}

fn simplify_in_runs(mesh: &TriMesh, tolerance: f32, run_triangles: usize) -> TriMesh {
    let n_tri = mesh.triangles.len();
    if n_tri == 0 || tolerance <= 0.0 {
        return mesh.clone();
    }
    let runs = n_tri.div_ceil(run_triangles);
    let run_len = n_tri.div_ceil(runs);

    // A vertex used by more than one run is a seam vertex: frozen.
    let mut owner = vec![u32::MAX; mesh.vertices.len()];
    let mut seam = vec![false; mesh.vertices.len()];
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let run = (t / run_len) as u32;
        for &v in tri {
            let o = &mut owner[v as usize];
            if *o == u32::MAX {
                *o = run;
            } else if *o != run {
                seam[v as usize] = true;
            }
        }
    }

    struct Piece {
        /// Global id of each local vertex.
        global: Vec<u32>,
        positions: Vec<[f32; 3]>,
        triangles: Vec<[u32; 3]>,
    }
    let pieces: Vec<Piece> = mesh
        .triangles
        .par_chunks(run_len)
        .map(|chunk| {
            let mut local: FxHashMap<u32, u32> = FxHashMap::default();
            let mut global = Vec::new();
            let mut vertices = Vec::new();
            let mut locked = Vec::new();
            let triangles: Vec<[u32; 3]> = chunk
                .iter()
                .map(|tri| {
                    tri.map(|v| {
                        *local.entry(v).or_insert_with(|| {
                            global.push(v);
                            vertices.push(mesh.vertices[v as usize]);
                            locked.push(seam[v as usize]);
                            (global.len() - 1) as u32
                        })
                    })
                })
                .collect();
            let (positions, triangles) = simplify_piece(&vertices, &triangles, locked, tolerance as f64);
            Piece { global, positions, triangles }
        })
        .collect();

    // Stitch: every surviving vertex keeps its global id (seam vertices are
    // shared between pieces and did not move), then unused ones are dropped.
    let mut out = TriMesh { vertices: mesh.vertices.clone(), triangles: Vec::new() };
    for piece in &pieces {
        for tri in &piece.triangles {
            for &v in tri {
                out.vertices[piece.global[v as usize] as usize] = piece.positions[v as usize];
            }
            out.triangles.push(tri.map(|v| piece.global[v as usize]));
        }
    }
    out.compact();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;
    use crate::isosurface;

    fn sphere(n: usize, r: f32) -> TriMesh {
        let grid = Grid { origin: [0.0; 3], spacing: [0.25; 3], dims: [n, n, n] };
        let c = (n as f32 - 1.0) / 2.0 * 0.25;
        let mut f = vec![0f32; n * n * n];
        for k in 0..n {
            for j in 0..n {
                for i in 0..n {
                    let d =
                        ((i as f32 * 0.25 - c).powi(2) + (j as f32 * 0.25 - c).powi(2) + (k as f32 * 0.25 - c).powi(2))
                            .sqrt();
                    f[grid.index(i, j, k)] = r - d;
                }
            }
        }
        isosurface::extract(&f, &grid)
    }

    #[test]
    fn sphere_stays_closed_and_keeps_its_volume() {
        let m = sphere(70, 7.0);
        assert!(m.edge_report().is_watertight());
        let s = simplify(&m, 0.03);
        assert!(s.edge_report().is_watertight(), "{:?}", s.edge_report());
        assert!(s.triangles.len() * 3 < m.triangles.len(), "{} -> {}", m.triangles.len(), s.triangles.len());
        let (v0, v1) = (m.signed_volume(), s.signed_volume());
        assert!((v0 - v1).abs() / v0 < 0.01, "{v0} vs {v1}");
        // Every vertex stays close to the sphere.
        let c = 69.0 / 2.0 * 0.25;
        for v in &s.vertices {
            let d = ((v[0] - c).powi(2) + (v[1] - c).powi(2) + (v[2] - c).powi(2)).sqrt();
            assert!((d - 7.0).abs() < 0.1, "vertex at radius {d}");
        }
    }

    #[test]
    fn flat_faces_collapse_to_almost_nothing() {
        let grid = Grid { origin: [0.0; 3], spacing: [0.5; 3], dims: [30, 30, 30] };
        let mut f = vec![-1f32; 27000];
        for k in 5..25 {
            for j in 5..25 {
                for i in 5..25 {
                    f[grid.index(i, j, k)] = 1.0;
                }
            }
        }
        let m = isosurface::extract(&f, &grid);
        let s = simplify(&m, 0.05);
        assert!(s.edge_report().is_watertight());
        assert!(s.triangles.len() < m.triangles.len() / 10, "{} -> {}", m.triangles.len(), s.triangles.len());
        assert!((s.signed_volume() - m.signed_volume()).abs() / m.signed_volume() < 0.02);
    }

    #[test]
    fn seams_between_parallel_runs_stay_closed() {
        let m = sphere(70, 7.0);
        assert!(m.triangles.len() > 5 * 4000);
        let s = simplify_in_runs(&m, 0.03, 4000);
        assert!(s.edge_report().is_watertight(), "{:?}", s.edge_report());
        assert!(s.triangles.len() < m.triangles.len() / 2);
    }

    #[test]
    fn zero_tolerance_is_a_no_op() {
        let m = sphere(20, 2.0);
        let s = simplify(&m, 0.0);
        assert_eq!(s.triangles.len(), m.triangles.len());
    }
}
