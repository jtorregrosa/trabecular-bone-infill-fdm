//! Small software renderer for preview images: orthographic projection,
//! z-buffer, flat shading, optional section cut with shaded cut faces.

use crate::mesh::TriMesh;
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::path::Path;

// The design tokens of src/web/theme.css: --canvas and --bone. Cut faces are
// the same bone, a little darker and unshaded, so the cut reads as a plane.
const BACKGROUND: [f32; 3] = [0.106, 0.106, 0.122];
const BONE: [f32; 3] = [0.929, 0.902, 0.839];
const CUT: [f32; 3] = [0.836, 0.812, 0.755];

/// A section cut: everything with `coordinate[axis] > value` is removed.
#[derive(Clone, Copy, Debug)]
pub struct Cut {
    pub axis: usize,
    pub value: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct View {
    /// Camera elevation above the XY plane and azimuth around Z, in degrees.
    /// `elevation = 90` looks straight down.
    pub elevation: f32,
    pub azimuth: f32,
    pub cut: Option<Cut>,
}

/// Rows of the matrix are the camera's right, up and towards-viewer axes.
fn camera(elevation: f32, azimuth: f32) -> [[f32; 3]; 3] {
    let (se, ce) = elevation.to_radians().sin_cos();
    let (sa, ca) = azimuth.to_radians().sin_cos();
    // Viewer direction (from the scene towards the eye).
    let back = [ce * ca, ce * sa, se];
    let right = [-sa, ca, 0.0];
    let up = [
        back[1] * right[2] - back[2] * right[1],
        back[2] * right[0] - back[0] * right[2],
        back[0] * right[1] - back[1] * right[0],
    ];
    [right, up, back]
}

fn apply(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Renders one view to an RGB8 buffer of `width * height` pixels.
pub fn render(mesh: &TriMesh, view: &View, width: usize, height: usize) -> Vec<u8> {
    const SS: usize = 2; // supersampling factor
    let (w, h) = (width * SS, height * SS);
    let cam = camera(view.elevation, view.azimuth);
    let projected: Vec<[f32; 3]> = mesh.vertices.par_iter().map(|&v| apply(&cam, v)).collect();

    // Fit the visible part in the frame.
    let visible = |v: &[f32; 3]| view.cut.is_none_or(|c| v[c.axis] <= c.value);
    let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for (p, v) in projected.iter().zip(&mesh.vertices) {
        if visible(v) {
            for a in 0..2 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
    }
    if lo[0] > hi[0] {
        return background(width, height);
    }
    let pad = 0.04;
    let scale = ((w as f32 * (1.0 - 2.0 * pad)) / (hi[0] - lo[0]).max(1e-6))
        .min((h as f32 * (1.0 - 2.0 * pad)) / (hi[1] - lo[1]).max(1e-6));
    let cx = 0.5 * (lo[0] + hi[0]);
    let cy = 0.5 * (lo[1] + hi[1]);
    let to_px = |p: [f32; 3]| [(p[0] - cx) * scale + 0.5 * w as f32, 0.5 * h as f32 - (p[1] - cy) * scale];

    let light = {
        let l = [-0.35f32, 0.45, 0.82];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };

    // Each band of rows is rasterised independently.
    let band_rows = 64usize;
    let mut color = vec![BACKGROUND; w * h];
    color.par_chunks_mut(w * band_rows).enumerate().for_each(|(band, rows)| {
        let y0 = band * band_rows;
        let y1 = y0 + rows.len() / w;
        let mut depth = vec![f32::NEG_INFINITY; rows.len()];
        for t in &mesh.triangles {
            let v = [mesh.vertices[t[0] as usize], mesh.vertices[t[1] as usize], mesh.vertices[t[2] as usize]];
            if let Some(c) = view.cut
                && v[0][c.axis] > c.value
                && v[1][c.axis] > c.value
                && v[2][c.axis] > c.value
            {
                continue;
            }
            let p = [projected[t[0] as usize], projected[t[1] as usize], projected[t[2] as usize]];
            let s = [to_px(p[0]), to_px(p[1]), to_px(p[2])];
            let ymin = s[0][1].min(s[1][1]).min(s[2][1]).floor().max(y0 as f32) as usize;
            let ymax = (s[0][1].max(s[1][1]).max(s[2][1]).ceil() as isize).min(y1 as isize - 1);
            if ymax < ymin as isize {
                continue;
            }
            let xmin = s[0][0].min(s[1][0]).min(s[2][0]).floor().max(0.0) as usize;
            let xmax = (s[0][0].max(s[1][0]).max(s[2][0]).ceil() as isize).min(w as isize - 1);
            if xmax < xmin as isize {
                continue;
            }
            let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
            if area == 0.0 {
                continue;
            }
            // Camera-space normal; z > 0 faces the viewer.
            let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let mut n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len == 0.0 {
                continue;
            }
            n = [n[0] / len, n[1] / len, n[2] / len];
            let front = n[2] > 0.0;
            if !front && view.cut.is_none() {
                continue; // closed surfaces never show their back
            }
            let shade = if front {
                let d = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
                let k = 0.22 + 0.78 * d;
                [BONE[0] * k, BONE[1] * k, BONE[2] * k]
            } else {
                CUT // the back of a surface seen through a cut is solid material
            };
            let inv = 1.0 / area;
            for y in ymin..=ymax as usize {
                let fy = y as f32 + 0.5;
                for x in xmin..=xmax as usize {
                    let fx = x as f32 + 0.5;
                    let w0 = ((s[1][0] - fx) * (s[2][1] - fy) - (s[2][0] - fx) * (s[1][1] - fy)) * inv;
                    let w1 = ((s[2][0] - fx) * (s[0][1] - fy) - (s[0][0] - fx) * (s[2][1] - fy)) * inv;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    if let Some(c) = view.cut {
                        let coord = w0 * v[0][c.axis] + w1 * v[1][c.axis] + w2 * v[2][c.axis];
                        if coord > c.value {
                            continue;
                        }
                    }
                    let z = w0 * p[0][2] + w1 * p[1][2] + w2 * p[2][2];
                    let idx = (y - y0) * w + x;
                    if z > depth[idx] {
                        depth[idx] = z;
                        rows[idx] = shade;
                    }
                }
            }
        }
    });

    // Box-filter down to the requested size.
    let mut out = vec![0u8; width * height * 3];
    out.par_chunks_mut(width * 3).enumerate().for_each(|(y, row)| {
        for x in 0..width {
            let mut acc = [0f32; 3];
            for sy in 0..SS {
                for sx in 0..SS {
                    let c = color[(y * SS + sy) * w + x * SS + sx];
                    for a in 0..3 {
                        acc[a] += c[a];
                    }
                }
            }
            for a in 0..3 {
                row[x * 3 + a] = (acc[a] / (SS * SS) as f32 * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    });
    out
}

fn background(width: usize, height: usize) -> Vec<u8> {
    let px = BACKGROUND.map(|c| (c * 255.0).round() as u8);
    let mut out = Vec::with_capacity(width * height * 3);
    for _ in 0..width * height {
        out.extend_from_slice(&px);
    }
    out
}

/// Writes several views side by side as one PNG.
pub fn write_views(mesh: &TriMesh, views: &[View], panel: (usize, usize), path: &Path) -> Result<()> {
    let (pw, ph) = panel;
    let width = pw * views.len();
    let mut image = vec![0u8; width * ph * 3];
    for (n, view) in views.iter().enumerate() {
        let panel = render(mesh, view, pw, ph);
        for y in 0..ph {
            let dst = (y * width + n * pw) * 3;
            image[dst..dst + pw * 3].copy_from_slice(&panel[y * pw * 3..(y + 1) * pw * 3]);
        }
    }
    write_png(path, width, ph, &image)
}

/// Writes an RGB8 image as PNG.
pub fn write_png(path: &Path, width: usize, height: usize, rgb: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    let file =
        std::io::BufWriter::new(std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?);
    let mut encoder = png::Encoder::new(file, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgb)?;
    Ok(())
}

/// Standard preview: three-quarter view, top view, and two section cuts
/// (vertical through the middle, horizontal through the middle).
pub fn write_preview(mesh: &TriMesh, path: &Path) -> Result<()> {
    let (lo, hi) = mesh.bounds();
    let mid = |a: usize| (0.5 * (lo[a] + hi[a])) as f32;
    let views = [
        View { elevation: 30.0, azimuth: -60.0, cut: None },
        View { elevation: 90.0, azimuth: -90.0, cut: None },
        View { elevation: 8.0, azimuth: 90.0, cut: Some(Cut { axis: 1, value: mid(1) }) },
        View { elevation: 90.0, azimuth: -90.0, cut: Some(Cut { axis: 2, value: mid(2) }) },
    ];
    write_views(mesh, &views, (800, 700), path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::box_mesh;

    #[test]
    fn box_covers_the_centre_and_leaves_background_at_corners() {
        let m = box_mesh([0.0; 3], [10.0; 3]);
        let img = render(&m, &View { elevation: 90.0, azimuth: -90.0, cut: None }, 64, 64);
        let px = |x: usize, y: usize| [img[(y * 64 + x) * 3], img[(y * 64 + x) * 3 + 1], img[(y * 64 + x) * 3 + 2]];
        assert!(px(32, 32)[0] > 150, "centre should be lit bone colour");
        let bg = BACKGROUND.map(|c| (c * 255.0).round() as u8);
        assert_eq!(px(0, 0), bg);
    }

    #[test]
    fn cut_reveals_the_inside() {
        let mut m = box_mesh([0.0; 3], [10.0; 3]);
        let mut cavity = box_mesh([2.0; 3], [8.0; 3]);
        cavity.flip();
        m.append(&cavity);
        let view = View { elevation: 0.0, azimuth: 90.0, cut: Some(Cut { axis: 1, value: 5.0 }) };
        let img = render(&m, &view, 80, 80);
        let px = |x: usize, y: usize| [img[(y * 80 + x) * 3], img[(y * 80 + x) * 3 + 1], img[(y * 80 + x) * 3 + 2]];
        let cut = CUT.map(|c| (c * 255.0).round() as u8);
        // The wall of the box is sectioned: cut colour near the edge...
        assert_eq!(px(10, 40), cut);
        // ...and the far wall of the cavity (lit bone) shows in the middle.
        assert_ne!(px(40, 40), cut);
        assert!(px(40, 40)[0] > 40);
    }
}
