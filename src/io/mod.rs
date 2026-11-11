//! Mesh file input and output (STL and 3MF).

pub mod stl;
pub mod threemf;

use crate::mesh::TriMesh;
use anyhow::{Context, Result, bail};
use std::path::Path;

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Reads an STL or 3MF file as an indexed mesh (vertices welded).
pub fn read_mesh(path: &Path) -> Result<TriMesh> {
    let mesh = match extension(path).as_str() {
        "stl" => stl::read(path),
        "3mf" => threemf::read(path),
        other => bail!("unsupported mesh format '.{other}' (expected .stl or .3mf)"),
    }
    .with_context(|| format!("reading {}", path.display()))?;
    if mesh.is_empty() {
        bail!("{} contains no triangles", path.display());
    }
    Ok(mesh)
}

/// Reads an STL or 3MF held in memory; `name` only supplies the extension.
pub fn read_mesh_bytes(name: &str, data: &[u8]) -> Result<TriMesh> {
    let mesh = match extension(Path::new(name)).as_str() {
        "stl" => stl::parse(data).map(|soup| TriMesh::from_soup(&soup)),
        "3mf" => threemf::read_from(std::io::Cursor::new(data)),
        other => bail!("unsupported mesh format '.{other}' (expected .stl or .3mf)"),
    }
    .with_context(|| format!("reading {name}"))?;
    if mesh.is_empty() {
        bail!("{name} contains no triangles");
    }
    Ok(mesh)
}

/// Encodes a mesh as STL (binary) or 3MF in memory; `format` is the
/// extension, `"stl"` or `"3mf"`.
pub fn write_mesh_bytes(format: &str, mesh: &TriMesh) -> Result<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    match format.to_ascii_lowercase().as_str() {
        "stl" => stl::write_to(&mut out, mesh)?,
        "3mf" => threemf::write_to(&mut out, mesh)?,
        other => bail!("unsupported output format '.{other}' (expected .stl or .3mf)"),
    }
    Ok(out.into_inner())
}

/// Writes an STL (binary) or 3MF file, chosen by extension.
pub fn write_mesh(path: &Path, mesh: &TriMesh) -> Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    match extension(path).as_str() {
        "stl" => stl::write(path, mesh),
        "3mf" => threemf::write(path, mesh),
        other => bail!("unsupported output format '.{other}' (expected .stl or .3mf)"),
    }
    .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::box_mesh;

    #[test]
    fn meshes_round_trip_through_memory() {
        let mesh = box_mesh([0.0; 3], [3.0, 4.0, 5.0]);
        for format in ["stl", "3mf"] {
            let bytes = write_mesh_bytes(format, &mesh).unwrap();
            let back = read_mesh_bytes(&format!("box.{format}"), &bytes).unwrap();
            assert_eq!(back.triangles.len(), 12, "{format}");
            assert!((back.signed_volume() - 60.0).abs() < 1e-6, "{format}");
        }
        assert!(write_mesh_bytes("obj", &mesh).is_err());
        assert!(read_mesh_bytes("box.obj", b"x").is_err());
        assert!(read_mesh_bytes("box.stl", b"not an stl").is_err());
    }
}
