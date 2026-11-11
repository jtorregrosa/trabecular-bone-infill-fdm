//! Renders the images and figures of the MakerWorld description
//! (`docs/makerworld/`) from the real models.
//!
//! ```text
//! cargo run --release --example sample_model      (once: writes the femur variant)
//! cargo run --release --example showcase
//! ```
//!
//! Every image is a render of what the generator actually produced, with the
//! crate's own software renderer and printability analyser. The figures it
//! prints are the ones quoted in the description. The photographic images
//! (cover, outside and inside, close-ups) are Blender renders of the same
//! files, made with the studio scene in `blender/`.

use anyhow::{Context, Result};
use bone_infill::analyze::{AnalyzeParams, analyze, layer_map};
use bone_infill::mesh::box_mesh;
use bone_infill::params::{Face, Params};
use bone_infill::pipeline::{generate, prepare_model};
use bone_infill::render::{Cut, View, render, write_png};
use bone_infill::{io, schema};
use std::path::Path;

/// Page background behind side-by-side panels (the canvas token).
const GAP_COLOUR: [u8; 3] = [27, 27, 31];
const GAP: usize = 24;

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("docs/makerworld/img");
    std::fs::create_dir_all(&out)?;
    let (model, _) = prepare_model(
        io::read_mesh(&root.join("models/femur_proximal_variant.stl"))
            .context("models/femur_proximal_variant.stl: run `cargo run --release --example sample_model` first")?,
    )?;
    let quiet = |_: &str| {};

    // The presets, exactly as the app applies them.
    let defaults = serde_json::to_value(Params::default())?;
    let preset = |name: &str| -> Result<Params> {
        let schema = schema::schema();
        let found = schema["presets"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap();
        let mut merged = defaults.clone();
        for (k, v) in found["params"].as_object().unwrap() {
            merged[k] = v.clone();
        }
        Ok(serde_json::from_value(merged)?)
    };

    let (lo, hi) = model.bounds();
    let mid_x = (0.5 * (lo[0] + hi[0])) as f32;
    // From +X, Z up: the cut face looks at the camera.
    let side_cut = View { elevation: 0.0, azimuth: 0.0, cut: Some(Cut { axis: 0, value: mid_x }) };

    // 6. Healthy against osteoporotic, after the classic pair of micrographs:
    //    a 60 mm block made only of lattice (every face open), once dense and
    //    fine with plates and small round holes, once sparse with wide pores
    //    and thin walls, seen from straight above and in grey; only the
    //    middle of the top face, away from the edges. Renders of the generator's
    //    lattice, not microscope images of bone.
    let block = box_mesh([0.0; 3], [60.0; 3]);
    let all_faces: Vec<Face> = (0..6).map(|i| Face { axis: i / 2, positive: i % 2 == 0 }).collect();
    let healthy = Params {
        density: 0.30,
        cell: 6.0,
        perforation: [0.20, 0.12],
        hole_ratio: 0.35,
        open: all_faces.clone(),
        ..Params::default()
    };
    let osteoporotic = Params { density: 0.09, cell: 10.0, open: all_faces, ..Params::default() };
    let close = View { elevation: 90.0, azimuth: -90.0, cut: None };
    let (w, h) = (1800, 1800);
    let mut panels = Vec::new();
    for (name, p) in [("healthy", &healthy), ("osteoporotic", &osteoporotic)] {
        let g = generate(&block, p, &quiet)?;
        println!(
            "{name}-looking block: lattice {:.1} %, walls {:.2} mm",
            100.0 * g.report.lattice_density,
            g.report.calibration.mean_wall
        );
        panels.push(micrograph(&render(&g.mesh, &close, w, h), w, h, MICROGRAPH_CROP));
    }
    save_row(&out.join("06_healthy_vs_osteoporotic.png"), &panels)?;
    if std::env::args().any(|a| a == "bones") {
        return Ok(());
    }

    let mut results = Vec::new();
    for name in ["Low", "Medium", "High"] {
        let g = generate(&model, &preset(name)?, &quiet)?;
        let r = &g.report;
        println!(
            "{name}: {:.0} g in PLA, {:.0} % lighter than solid, lattice {:.1} %, walls {:.2} mm, {:.1} s",
            r.weight_pla_g,
            100.0 * (1.0 - r.volume_fraction),
            100.0 * r.lattice_density,
            r.calibration.mean_wall,
            r.total_seconds
        );
        results.push((name, g.mesh));
    }
    let solid_g = model.signed_volume() / 1000.0 * 1.24;
    println!("solid: {solid_g:.0} g in PLA");
    let medium = &results[1].1;

    // 3. Low, Medium, High.
    let panels: Vec<_> = results.iter().map(|(_, m)| (render(m, &side_cut, 640, 800), 640, 800)).collect();
    save_row(&out.join("03_presets.png"), &panels)?;

    // 4. Printability: the same lattice with the printing rules switched off
    //    (pores as long in every direction, no overhang limit, no vaults, no
    //    thin-feature removal) against ours, both judged by the analyser at
    //    45 degrees and 4 mm bridges. Not a model of real bone: the generator
    //    without its rules.
    let naive_params =
        Params { z_elongation: 1.0, max_overhang_deg: 85.0, vaults: false, min_feature: 0.0, ..preset("Medium")? };
    let naive = generate(&model, &naive_params, &quiet)?.mesh;
    let check = AnalyzeParams::default();
    let naive_report = analyze(&naive, Some(&model), &check);
    let ours_report = analyze(medium, Some(&model), &check);
    for (label, r) in [("without rules", &naive_report), ("with the rules", &ours_report)] {
        println!(
            "printability {label}: {:.0} mm2 unsupported inside ({:.2} %), {} islands in mid-air ({:.1} mm2)",
            r.unsupported_interior_area, r.unsupported_interior_pct, r.interior_islands, r.interior_island_area
        );
    }
    let z = naive_report.worst_layers.first().map_or(0.5 * (lo[2] + hi[2]), |l| l.0);
    let maps: Vec<_> = [&naive, medium]
        .iter()
        .map(|m| {
            let map = layer_map(m, Some(&model), &check, z);
            crop_and_enlarge(&map.rgb, map.width, map.height, 16, 3)
        })
        .collect();
    println!("layer maps at z = {z:.1} mm");
    save_row(&out.join("04_printability.png"), &maps)?;

    println!("images: {}", out.display());
    Ok(())
}

/// The part of an image that differs from its corner colour, with a margin,
/// enlarged `scale` times (nearest neighbour, so the raster stays crisp).
fn crop_and_enlarge(rgb: &[u8], width: usize, height: usize, margin: usize, scale: usize) -> (Vec<u8>, usize, usize) {
    let background = &rgb[0..3];
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
    for y in 0..height {
        for x in 0..width {
            if &rgb[3 * (y * width + x)..3 * (y * width + x) + 3] != background {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    if x0 > x1 {
        return (rgb.to_vec(), width, height);
    }
    let (x0, y0) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
    let (x1, y1) = ((x1 + margin).min(width - 1), (y1 + margin).min(height - 1));
    let (w, h) = ((x1 - x0 + 1) * scale, (y1 - y0 + 1) * scale);
    let mut out = vec![0u8; 3 * w * h];
    for y in 0..h {
        for x in 0..w {
            let src = 3 * ((y0 + y / scale) * width + x0 + x / scale);
            out[3 * (y * w + x)..3 * (y * w + x) + 3].copy_from_slice(&rgb[src..src + 3]);
        }
    }
    (out, w, h)
}

/// Part of the full render kept for the close-ups, as fractions of its width
/// and height: left, top, right, bottom.
const MICROGRAPH_CROP: [f64; 4] = [0.2, 0.2, 0.8, 0.62];

/// Crops a render and turns it into a grey, high-contrast close-up in the
/// manner of an electron micrograph (pores dark, bone bright).
fn micrograph(rgb: &[u8], width: usize, height: usize, crop: [f64; 4]) -> (Vec<u8>, usize, usize) {
    let x0 = (crop[0] * width as f64) as usize;
    let y0 = (crop[1] * height as f64) as usize;
    let x1 = (crop[2] * width as f64) as usize;
    let y1 = (crop[3] * height as f64) as usize;
    let (w, h) = (x1 - x0, y1 - y0);
    let mut out = vec![0u8; 3 * w * h];
    for y in 0..h {
        for x in 0..w {
            let i = 3 * ((y0 + y) * width + x0 + x);
            let l = 0.30 * rgb[i] as f64 + 0.59 * rgb[i + 1] as f64 + 0.11 * rgb[i + 2] as f64;
            let v = ((l - 28.0) / 210.0).clamp(0.0, 1.0).powf(0.85) * 255.0;
            out[3 * (y * w + x)..3 * (y * w + x) + 3].fill(v as u8);
        }
    }
    (out, w, h)
}

/// Panels side by side, centred vertically, with a gap between them.
fn save_row(path: &Path, panels: &[(Vec<u8>, usize, usize)]) -> Result<()> {
    let height = panels.iter().map(|p| p.2).max().unwrap_or(0);
    let width = panels.iter().map(|p| p.1).sum::<usize>() + GAP * panels.len().saturating_sub(1);
    let mut rgb = GAP_COLOUR.repeat(width * height);
    let mut x0 = 0;
    for (pixels, w, h) in panels {
        let y0 = (height - h) / 2;
        for y in 0..*h {
            let src = &pixels[3 * y * w..3 * (y + 1) * w];
            let at = 3 * ((y0 + y) * width + x0);
            rgb[at..at + 3 * w].copy_from_slice(src);
        }
        x0 += w + GAP;
    }
    write_png(path, width, height, &rgb)
}
