//! Command-line interface.

use anyhow::{Context, Result, bail};
use bone_infill::analyze::{AnalyzeParams, PrintabilityReport, analyze};
use bone_infill::params::{Face, Load, Params};
use bone_infill::viewer::{ViewerInfo, write_viewer};
use bone_infill::{io, pipeline};
use clap::{Args, Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "bone-infill", version, about = "Fill closed 3D models with a printable trabecular-bone lattice")]
struct Cli {
    /// Worker threads (default: all cores).
    #[arg(long, global = true)]
    threads: Option<usize>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate the bone infill for a model.
    Generate(Box<GenerateArgs>),
    /// Check how printable a mesh is without internal supports.
    Analyze(AnalyzeArgs),
    /// Write an interactive 3D viewer (one self-contained .html) for a mesh.
    View(ViewArgs),
    /// Start the local web app: load a model, tune parameters, download.
    Serve(ServeArgs),
    /// Cut a flat slab out of a model, to fill it with --open=+z,-z.
    Section(SectionArgs),
}

#[derive(Args)]
struct SectionArgs {
    /// Closed model to cut (.stl or .3mf).
    mesh: PathBuf,
    /// Output file [default: out/<model>_section.stl]
    #[arg(short, long)]
    out: Option<PathBuf>,
    /// A point "X,Y,Z" the middle of the slab passes through
    /// [default: the centre of the model]
    #[arg(long, value_parser = parse_vec3, allow_hyphen_values = true)]
    through: Option<[f64; 3]>,
    /// Normal "A,B,C" of the cutting plane; it becomes Z of the slab.
    #[arg(long, value_parser = parse_vec3, allow_hyphen_values = true, default_value = "0,1,0")]
    normal: [f64; 3],
    /// Direction "A,B,C" that becomes Y of the slab.
    #[arg(long, value_parser = parse_vec3, allow_hyphen_values = true, default_value = "0,0,1")]
    up: [f64; 3],
    /// Slab thickness (mm).
    #[arg(long, default_value_t = 18.0)]
    thickness: f64,
    /// Keep only this many mm below the --through point, along --up.
    #[arg(long)]
    below: Option<f64>,
    /// Keep only this many mm above the --through point, along --up.
    #[arg(long)]
    above: Option<f64>,
    /// Scale the model by this factor before cutting.
    #[arg(long, default_value_t = 1.0)]
    scale: f64,
    /// Sampling pitch (mm).
    #[arg(long, default_value_t = 0.25)]
    voxel: f64,
}

#[derive(Args)]
struct ServeArgs {
    /// Port to listen on (127.0.0.1 only).
    #[arg(long, default_value_t = 8787)]
    port: u16,
    /// Do not open the browser.
    #[arg(long)]
    no_open: bool,
}

#[derive(Args)]
struct GenerateArgs {
    /// Closed model, STL (binary or ASCII) or 3MF. Z is the print direction.
    model: PathBuf,
    /// Output file, .3mf or .stl [default: out/<model>_bone.3mf]
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Thickness of the solid skin (mm).
    #[arg(long, default_value_t = Params::default().shell)]
    shell: f64,
    /// Cell size of the lattice (mm). Smaller is finer.
    #[arg(long, default_value_t = Params::default().cell)]
    cell: f64,
    /// Solid fraction of the lattice region.
    #[arg(long, default_value_t = Params::default().density)]
    density: f64,
    /// Faces left without skin to show the lattice, e.g. --open=+z,-z
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    open: Vec<Face>,
    /// Reinforced sphere "X,Y,Z,R" in mm (repeatable).
    #[arg(long, value_parser = parse_load, allow_hyphen_values = true)]
    load: Vec<Load>,
    /// Main load direction "DX,DY,DZ": pores are elongated along it.
    #[arg(long, value_parser = parse_vec3, allow_hyphen_values = true)]
    stretch_dir: Option<[f64; 3]>,
    /// Elongation factor along --stretch-dir.
    #[arg(long, default_value_t = Params::default().stretch)]
    stretch: f64,
    /// Pore elongation along Z. Higher gives steeper, more printable sheets.
    #[arg(long, default_value_t = Params::default().z_elongation)]
    z_elongation: f64,
    /// Extra density next to the skin (0 = uniform).
    #[arg(long, default_value_t = Params::default().skin_boost)]
    skin_boost: f64,
    /// Minimum sheet thickness (mm).
    #[arg(long, default_value_t = Params::default().min_wall)]
    min_wall: f64,
    /// Size of the perforations relative to the cell.
    #[arg(long, default_value_t = Params::default().hole_ratio)]
    hole_ratio: f64,
    /// Hole elongation along Z (1 = round holes)
    #[arg(long, default_value_t = Params::default().hole_elongation)]
    hole_elongation: f64,
    /// Perforated share of the sheets "SPARSE,DENSE": where the lattice is
    /// sparsest and where it is densest. Higher gives rods, lower plates.
    #[arg(long, value_parser = parse_pair, default_value = "0.39,0.27")]
    perforation: [f64; 2],
    /// Irregularity of the pattern: 0 is almost periodic, 1 very irregular.
    #[arg(long, default_value_t = Params::default().bandwidth)]
    bandwidth: f64,
    /// Fillet radius where lattice and skin merge (mm).
    #[arg(long, default_value_t = Params::default().blend)]
    blend: f64,
    /// How much denser the lattice gets inside a --load region.
    #[arg(long, default_value_t = Params::default().load_boost)]
    load_boost: f64,

    /// Steepest lattice overhang allowed, degrees from vertical.
    #[arg(long, default_value_t = Params::default().max_overhang_deg)]
    max_overhang: f64,
    /// Longest bridge allowed (mm); both ends must rest on supported material.
    #[arg(long, default_value_t = Params::default().max_bridge)]
    max_bridge: f64,
    /// Narrowest band of bridge strands accepted as a bridge (mm).
    #[arg(long, default_value_t = Params::default().min_bridge_width)]
    bridge_width: f64,
    /// Do not build vaults under inner ceilings.
    #[arg(long)]
    no_vaults: bool,
    /// Width of the slots vaults leave under flat roofs, as a share of
    /// --max-bridge. Lower closes roofs more, at the cost of material.
    #[arg(long, default_value_t = Params::default().vault_slot)]
    vault_slot: f64,
    /// Remove lattice features thinner than this (mm); 0 disables.
    #[arg(long, default_value_t = Params::default().min_feature)]
    min_feature: f64,
    /// Do not regenerate when the printability passes miss the density.
    #[arg(long)]
    no_density_compensation: bool,

    /// Voxel size (mm).
    #[arg(long, default_value_t = Params::default().voxel)]
    voxel: f64,
    /// Voxel budget in millions; larger parts get a coarser voxel.
    #[arg(long, default_value_t = Params::default().max_voxels / 1e6)]
    max_voxels: f64,
    /// Plane waves per noise field.
    #[arg(long, default_value_t = Params::default().waves)]
    waves: usize,
    /// Mesh simplification tolerance (mm); 0 disables [default: voxel / 4]
    #[arg(long)]
    simplify: Option<f64>,
    /// Pattern seed.
    #[arg(long, default_value_t = Params::default().seed)]
    seed: u64,

    /// Write the generation and printability report as JSON.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Skip the printability check of the result.
    #[arg(long)]
    no_check: bool,
    /// Layer height assumed by the printability check (mm).
    #[arg(long, default_value_t = AnalyzeParams::default().layer_height)]
    layer_height: f64,
    /// Mid-air regions smaller than this (mm2) are reported as specks.
    #[arg(long, default_value_t = AnalyzeParams::default().min_island_area)]
    min_island_area: f64,
    /// Also write a preview image next to the output (same name, .png).
    #[arg(long)]
    preview: bool,
    /// Also write an interactive 3D viewer next to the output (same name,
    /// .html): orbit, zoom, section along X/Y/Z, overhang colouring.
    #[arg(long)]
    viewer: bool,
}

#[derive(Args)]
struct ViewArgs {
    /// Mesh to show (.stl or .3mf).
    mesh: PathBuf,
    /// Output page [default: the mesh path with .html]
    #[arg(short, long)]
    out: Option<PathBuf>,
    /// Layer height, so the Z section can report the layer number (mm).
    #[arg(long, default_value_t = AnalyzeParams::default().layer_height)]
    layer_height: f64,
}

#[derive(Args)]
struct AnalyzeArgs {
    /// Mesh to analyse (.stl or .3mf). Z is the print direction.
    mesh: PathBuf,
    /// Original model before infill: separates the model's own overhangs
    /// from the ones the infill introduces.
    #[arg(long)]
    reference: Option<PathBuf>,
    #[arg(long, default_value_t = AnalyzeParams::default().layer_height)]
    layer_height: f64,
    /// XY raster size (mm).
    #[arg(long, default_value_t = AnalyzeParams::default().resolution)]
    resolution: f64,
    /// Overhang limit, degrees from vertical.
    #[arg(long, default_value_t = AnalyzeParams::default().max_overhang_deg)]
    max_overhang: f64,
    /// Bridge limit (mm).
    #[arg(long, default_value_t = AnalyzeParams::default().max_bridge)]
    max_bridge: f64,
    /// Narrowest band of bridge strands accepted as a bridge (mm).
    #[arg(long, default_value_t = AnalyzeParams::default().min_bridge_width)]
    bridge_width: f64,
    /// Mid-air regions smaller than this (mm2) are reported as specks.
    #[arg(long, default_value_t = AnalyzeParams::default().min_island_area)]
    min_island_area: f64,
    /// Skin thickness (mm) used when the part was generated: splits the
    /// unsupported area into skin (inner ceilings) and lattice.
    #[arg(long)]
    shell: Option<f64>,
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
    /// Write a picture (.png) of one layer showing what is unsupported.
    #[arg(long)]
    map: Option<PathBuf>,
    /// Height of the layer to draw [default: the worst layer]
    #[arg(long)]
    map_z: Option<f64>,
}

fn parse_numbers<const N: usize>(s: &str) -> Result<[f64; N]> {
    let parts: Vec<f64> = s
        .split(',')
        .map(|v| v.trim().parse::<f64>().with_context(|| format!("'{v}' is not a number")))
        .collect::<Result<_>>()?;
    match <[f64; N]>::try_from(parts) {
        Ok(a) => Ok(a),
        Err(v) => bail!("expected {N} comma-separated numbers, got {}", v.len()),
    }
}

fn parse_pair(s: &str) -> Result<[f64; 2]> {
    parse_numbers::<2>(s)
}

fn parse_vec3(s: &str) -> Result<[f64; 3]> {
    parse_numbers::<3>(s)
}

fn parse_load(s: &str) -> Result<Load> {
    let [x, y, z, r] = parse_numbers::<4>(s)?;
    Ok(Load { center: [x, y, z], radius: r })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(n) = cli.threads {
        rayon::ThreadPoolBuilder::new().num_threads(n).build_global().context("setting up the thread pool")?;
    }
    match cli.command {
        Command::Generate(args) => run_generate(*args),
        Command::Analyze(args) => run_analyze(args),
        Command::View(args) => run_view(args),
        Command::Section(args) => run_section(args),
        Command::Serve(args) => bone_infill::serve::serve(args.port, !args.no_open),
    }
}

fn run_generate(a: GenerateArgs) -> Result<()> {
    let params = Params {
        shell: a.shell,
        cell: a.cell,
        density: a.density,
        open: a.open,
        loads: a.load,
        stretch_dir: a.stretch_dir,
        stretch: a.stretch,
        z_elongation: a.z_elongation,
        skin_boost: a.skin_boost,
        min_wall: a.min_wall,
        hole_ratio: a.hole_ratio,
        hole_elongation: a.hole_elongation,
        perforation: a.perforation,
        bandwidth: a.bandwidth,
        blend: a.blend,
        load_boost: a.load_boost,
        min_bridge_width: a.bridge_width,
        vault_slot: a.vault_slot,
        max_overhang_deg: a.max_overhang,
        max_bridge: a.max_bridge,
        vaults: !a.no_vaults,
        min_feature: a.min_feature,
        compensate_density: !a.no_density_compensation,
        voxel: a.voxel,
        max_voxels: a.max_voxels * 1e6,
        waves: a.waves,
        simplify: a.simplify,
        seed: a.seed,
    };
    params.validate()?;
    let out = a.out.unwrap_or_else(|| {
        let stem = a.model.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
        Path::new("out").join(format!("{stem}_bone.3mf"))
    });

    let (model, notes) = pipeline::prepare_model(io::read_mesh(&a.model)?)?;
    for n in &notes {
        println!("note: {n}");
    }
    let generated = pipeline::generate(&model, &params, &|line| println!("{line}"))?;
    let r = &generated.report;

    let t = std::time::Instant::now();
    io::write_mesh(&out, &generated.mesh)?;
    let write_seconds = t.elapsed().as_secs_f64();
    let size_mb = std::fs::metadata(&out).map(|m| m.len() as f64 / 1048576.0).unwrap_or(0.0);

    println!();
    println!(
        "lattice: {:.1} % solid (target {:.1} %), walls {:.2} mm on average{}",
        100.0 * r.lattice_density,
        100.0 * r.params.density,
        r.calibration.mean_wall,
        if r.calibration.wall_clamped { " (clamped by --min-wall)" } else { "" }
    );
    println!(
        "printability passes: carved {:.0} mm3 unsupported, {:.0} mm3 thin, {:.0} mm3 loose; kept {:.0} mm3 as bridges; added {:.0} mm3 of vaults",
        r.removed_unsupported, r.removed_thin, r.removed_loose, r.bridged, r.vaults
    );
    println!(
        "mesh: {} triangles ({} before simplification), {}",
        r.triangles,
        r.triangles_before_simplify,
        if r.topology.is_watertight() {
            "watertight".to_string()
        } else {
            format!(
                "NOT watertight ({} open, {} non-manifold, {} misoriented edges)",
                r.topology.boundary, r.topology.non_manifold, r.topology.misoriented
            )
        }
    );
    println!(
        "volume: {:.1} cm3 ({:.0} % of the solid model), about {:.0} g in PLA",
        r.volume / 1000.0,
        100.0 * r.volume_fraction,
        r.weight_pla_g
    );
    println!(
        "output: {} ({size_mb:.1} MB) in {:.1} s (+ {write_seconds:.1} s writing)",
        out.display(),
        r.total_seconds
    );

    let mut printability = None;
    if !a.no_check {
        let t = std::time::Instant::now();
        let check = AnalyzeParams {
            layer_height: a.layer_height,
            max_overhang_deg: params.max_overhang_deg,
            max_bridge: params.max_bridge,
            skin: Some(params.shell),
            min_bridge_width: params.min_bridge_width,
            min_island_area: a.min_island_area,
            ..AnalyzeParams::default()
        };
        let report = analyze(&generated.mesh, Some(&model), &check);
        println!();
        print_printability(&report);
        println!("(checked in {:.1} s)", t.elapsed().as_secs_f64());
        printability = Some(report);
    }

    if let Some(path) = a.report {
        #[derive(serde::Serialize)]
        struct Full<'a> {
            generation: &'a pipeline::GenerateReport,
            printability: Option<&'a PrintabilityReport>,
        }
        let json = serde_json::to_string_pretty(&Full { generation: r, printability: printability.as_ref() })?;
        std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
        println!("report: {}", path.display());
    }

    if a.preview {
        let path = out.with_extension("png");
        bone_infill::render::write_preview(&generated.mesh, &path)?;
        println!("preview: {}", path.display());
    }
    if a.viewer {
        let path = out.with_extension("html");
        let mut rows = vec![
            ("Lattice density".to_string(), format!("{:.1} %", 100.0 * r.lattice_density)),
            ("Mean wall".to_string(), format!("{:.2} mm", r.calibration.mean_wall)),
            ("Volume".to_string(), format!("{:.1} cm3 ({:.0} %)", r.volume / 1000.0, 100.0 * r.volume_fraction)),
            ("Weight in PLA".to_string(), format!("{:.0} g", r.weight_pla_g)),
        ];
        if let Some(check) = &printability {
            rows.push(("Unsupported inside".to_string(), format!("{:.1} mm2", check.unsupported_interior_area)));
            rows.push(("Islands".to_string(), check.interior_islands.to_string()));
            if let Some((z, area)) = check.worst_layers.first() {
                rows.push(("Worst layer".to_string(), format!("z = {z:.1} mm ({area:.1} mm2)")));
            }
        }
        let title = out.file_stem().and_then(|s| s.to_str()).unwrap_or("model").to_string();
        write_viewer(&generated.mesh, &path, &ViewerInfo { title, layer_height: Some(a.layer_height), rows })?;
        println!("viewer: {}", path.display());
    }
    Ok(())
}

fn run_section(a: SectionArgs) -> Result<()> {
    let (model, notes) = pipeline::prepare_model(io::read_mesh(&a.mesh)?)?;
    for n in &notes {
        println!("note: {n}");
    }
    let (lo, hi) = model.bounds();
    let params = bone_infill::section::SectionParams {
        through: a.through.unwrap_or([0, 1, 2].map(|i| 0.5 * (lo[i] + hi[i]))),
        normal: a.normal,
        up: a.up,
        thickness: a.thickness,
        below: a.below,
        above: a.above,
        scale: a.scale,
        voxel: a.voxel,
    };
    let slab = bone_infill::section::section(&model, &params)?;
    let out = a.out.unwrap_or_else(|| {
        let stem = a.mesh.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
        Path::new("out").join(format!("{stem}_section.stl"))
    });
    io::write_mesh(&out, &slab)?;
    let (_, size) = slab.bounds();
    println!(
        "section: {} ({:.1} x {:.1} x {:.1} mm, {:.1} cm3, {} triangles)",
        out.display(),
        size[0],
        size[1],
        size[2],
        slab.signed_volume() / 1000.0,
        slab.triangles.len()
    );
    println!("fill it with: bone-infill generate {} --open=+z,-z", out.display());
    Ok(())
}

fn run_view(a: ViewArgs) -> Result<()> {
    let mesh = io::read_mesh(&a.mesh)?;
    let out = a.out.unwrap_or_else(|| a.mesh.with_extension("html"));
    let topology = mesh.edge_report();
    let rows = vec![
        ("Volume".to_string(), format!("{:.1} cm3", mesh.signed_volume() / 1000.0)),
        ("Closed surface".to_string(), if topology.is_watertight() { "yes" } else { "no" }.to_string()),
    ];
    let title = a.mesh.file_stem().and_then(|s| s.to_str()).unwrap_or("model").to_string();
    write_viewer(&mesh, &out, &ViewerInfo { title, layer_height: Some(a.layer_height), rows })?;
    println!("viewer: {}", out.display());
    Ok(())
}

fn run_analyze(a: AnalyzeArgs) -> Result<()> {
    let mesh = io::read_mesh(&a.mesh)?;
    let reference = a.reference.as_deref().map(io::read_mesh).transpose()?;
    let params = AnalyzeParams {
        layer_height: a.layer_height,
        resolution: a.resolution,
        max_overhang_deg: a.max_overhang,
        max_bridge: a.max_bridge,
        skin: a.shell,
        min_bridge_width: a.bridge_width,
        min_island_area: a.min_island_area,
        ..AnalyzeParams::default()
    };
    let report = analyze(&mesh, reference.as_ref(), &params);
    if a.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        let topology = mesh.edge_report();
        println!(
            "{}: {} triangles, {:.1} cm3, {}",
            a.mesh.display(),
            mesh.triangles.len(),
            mesh.signed_volume() / 1000.0,
            if topology.is_watertight() { "watertight" } else { "not watertight" }
        );
        print_printability(&report);
    }
    if let Some(path) = a.map {
        let z = a.map_z.or(report.worst_layers.first().map(|l| l.0));
        match z {
            Some(z) => {
                let map = bone_infill::analyze::layer_map(&mesh, reference.as_ref(), &params, z);
                bone_infill::render::write_png(&path, map.width, map.height, &map.rgb)?;
                eprintln!("layer map at z = {:.2} mm: {}", map.z, path.display());
            }
            None => eprintln!("nothing unsupported: no layer map written"),
        }
    }
    Ok(())
}

fn print_printability(r: &PrintabilityReport) {
    println!(
        "printability at {:.2} mm layers (limits: {:.0} deg overhang, {:.1} mm bridges; raster {:.2} mm):",
        r.layer_height, r.max_overhang_deg, r.max_bridge, r.resolution
    );
    println!(
        "  unsupported inside : {:8.1} mm2  ({:.3} % of all printed area) in {} layers",
        r.unsupported_interior_area, r.unsupported_interior_pct, r.layers
    );
    if let (Some(skin), Some(lattice)) = (r.unsupported_skin_area, r.unsupported_lattice_area) {
        println!("      of which skin  : {skin:8.1} mm2  (inner ceilings), lattice: {lattice:.1} mm2");
    }
    println!(
        "  islands in mid-air : {:8}      ({:.1} mm2; plus {} specks too small to extrude)",
        r.interior_islands, r.interior_island_area, r.interior_specks
    );
    println!(
        "  bridges            : {:8.1} mm2  (longest {:.1} mm; widest gap too long to bridge {:.1} mm)",
        r.bridge_area, r.longest_bridge, r.longest_rejected_span
    );
    println!("  gentle overhangs   : {:8.1} mm2", r.gentle_overhang_area);
    println!("  model's own overhangs (need external supports): {:.1} mm2", r.unsupported_exterior_area);
    let h = &r.down_facing_area_by_angle;
    let total: f64 = h.iter().sum();
    if total > 0.0 {
        println!(
            "  down-facing inner surface by overhang angle: <30: {:.0} %, 30-45: {:.0} %, 45-60: {:.0} %, 60-75: {:.0} %, >75: {:.0} %  ({:.0} cm2)",
            100.0 * h[0] / total,
            100.0 * h[1] / total,
            100.0 * h[2] / total,
            100.0 * h[3] / total,
            100.0 * h[4] / total,
            total / 100.0
        );
    }
    for e in r.island_examples.iter().take(3) {
        println!("  island of {:.2} mm2 at x = {:.1}, y = {:.1}, z = {:.1}", e[3], e[0], e[1], e[2]);
    }
    if let Some((z, area)) = r.worst_layers.first() {
        println!("  worst layer: z = {z:.1} mm with {area:.1} mm2 unsupported");
    }
}
