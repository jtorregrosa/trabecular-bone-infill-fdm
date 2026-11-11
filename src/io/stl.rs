//! STL reader (binary and ASCII) and binary writer.

use crate::mesh::{TriMesh, area_normal};
use anyhow::{Result, bail};
use rayon::prelude::*;
use std::io::Write;
use std::path::Path;

pub fn read(path: &Path) -> Result<TriMesh> {
    let data = std::fs::read(path)?;
    Ok(TriMesh::from_soup(&parse(&data)?))
}

/// Parses STL bytes into a triangle soup.
pub fn parse(data: &[u8]) -> Result<Vec<[[f32; 3]; 3]>> {
    // A binary STL is recognised by its exact size; ASCII files also start
    // with "solid" but so do many binary ones, so size is the reliable test.
    if data.len() >= 84 {
        let n = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
        if data.len() == 84 + 50 * n {
            return Ok(data[84..]
                .par_chunks_exact(50)
                .map(|rec| {
                    let mut tri = [[0f32; 3]; 3];
                    for (v, vertex) in tri.iter_mut().enumerate() {
                        for (c, coord) in vertex.iter_mut().enumerate() {
                            let o = 12 + v * 12 + c * 4;
                            *coord = f32::from_le_bytes(rec[o..o + 4].try_into().unwrap());
                        }
                    }
                    tri
                })
                .collect());
        }
    }
    parse_ascii(data)
}

fn parse_ascii(data: &[u8]) -> Result<Vec<[[f32; 3]; 3]>> {
    let text = String::from_utf8_lossy(data);
    let mut coords: Vec<f32> = Vec::new();
    let mut tokens = text.split_ascii_whitespace();
    while let Some(tok) = tokens.next() {
        if tok == "vertex" {
            for _ in 0..3 {
                match tokens.next().and_then(|t| t.parse::<f32>().ok()) {
                    Some(v) => coords.push(v),
                    None => bail!("malformed ASCII STL: bad vertex coordinate"),
                }
            }
        }
    }
    if coords.is_empty() || !coords.len().is_multiple_of(9) {
        bail!("not a valid STL file (neither binary nor ASCII)");
    }
    Ok(coords.as_chunks::<9>().0.iter().map(|c| [[c[0], c[1], c[2]], [c[3], c[4], c[5]], [c[6], c[7], c[8]]]).collect())
}

pub fn write(path: &Path, mesh: &TriMesh) -> Result<()> {
    write_to(&mut std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(path)?), mesh)
}

/// Writes a binary STL to any writer.
pub fn write_to(out: &mut impl Write, mesh: &TriMesh) -> Result<()> {
    if mesh.triangles.len() > u32::MAX as usize {
        bail!("too many triangles for an STL file");
    }
    let mut header = [b' '; 80];
    let tag = b"bone-infill";
    header[..tag.len()].copy_from_slice(tag);
    out.write_all(&header)?;
    out.write_all(&(mesh.triangles.len() as u32).to_le_bytes())?;
    for batch in mesh.triangles.chunks(1 << 18) {
        let bytes: Vec<u8> = batch
            .par_iter()
            .flat_map_iter(|t| {
                let n = area_normal(&mesh.vertices, t);
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                let inv = if len > 0.0 { 1.0 / len } else { 0.0 };
                let mut rec = [0u8; 50];
                for c in 0..3 {
                    rec[c * 4..c * 4 + 4].copy_from_slice(&((n[c] * inv) as f32).to_le_bytes());
                }
                for v in 0..3 {
                    let p = mesh.vertices[t[v] as usize];
                    for c in 0..3 {
                        let o = 12 + v * 12 + c * 4;
                        rec[o..o + 4].copy_from_slice(&p[c].to_le_bytes());
                    }
                }
                rec
            })
            .collect();
        out.write_all(&bytes)?;
    }
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::box_mesh;

    #[test]
    fn binary_round_trip() {
        let dir = std::env::temp_dir().join(format!("bone-infill-stl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("box.stl");
        let m = box_mesh([0.0; 3], [1.0, 2.0, 3.0]);
        write(&path, &m).unwrap();
        let back = read(&path).unwrap();
        assert_eq!(back.vertices.len(), 8);
        assert_eq!(back.triangles.len(), 12);
        assert!((back.signed_volume() - 6.0).abs() < 1e-6);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ascii_is_parsed() {
        let text = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        let soup = parse(text.as_bytes()).unwrap();
        assert_eq!(soup.len(), 1);
        assert_eq!(soup[0][1], [1.0, 0.0, 0.0]);
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(parse(b"hello world").is_err());
    }
}
