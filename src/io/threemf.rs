//! Minimal 3MF writer and reader.
//!
//! The writer emits the three parts of the core specification (content types,
//! relationships, one model with one mesh object in millimetres), which is
//! what Bambu Studio, OrcaSlicer and PrusaSlicer import as plain geometry.
//!
//! The reader collects the triangles of every `<mesh>` in every `.model` part.
//! It ignores build transforms and component instancing, so it is suitable
//! for single-object files such as the ones this tool writes.

use crate::mesh::TriMesh;
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use std::fmt::Write as _;
use std::io::{Read, Seek, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const CONTENT_TYPES: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
    "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
    "<Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>",
    "</Types>"
);

const RELS: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    "<Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" ",
    "Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>",
    "</Relationships>"
);

const MODEL_HEAD: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<model unit=\"millimeter\" xml:lang=\"en-US\" ",
    "xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n",
    "<metadata name=\"Application\">bone-infill</metadata>\n",
    "<resources><object id=\"1\" type=\"model\"><mesh><vertices>\n"
);

const MODEL_TAIL: &str = "</triangles></mesh></object></resources>\n<build><item objectid=\"1\"/></build>\n</model>\n";

/// Elements formatted per parallel task, and tasks per write batch.
const CHUNK: usize = 1 << 15;
const BATCH: usize = 64;

pub fn write(path: &Path, mesh: &TriMesh) -> Result<()> {
    write_to(std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(path)?), mesh)
}

/// Writes a 3MF package to any seekable writer.
pub fn write_to<W: Write + Seek>(writer: W, mesh: &TriMesh) -> Result<()> {
    let mut zip = ZipWriter::new(writer);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(4))
        .large_file(true);

    zip.start_file("[Content_Types].xml", options)?;
    zip.write_all(CONTENT_TYPES.as_bytes())?;
    zip.start_file("_rels/.rels", options)?;
    zip.write_all(RELS.as_bytes())?;

    zip.start_file("3D/3dmodel.model", options)?;
    zip.write_all(MODEL_HEAD.as_bytes())?;
    // Shortest round-trip float formatting keeps the input surface bit-exact.
    for batch in mesh.vertices.chunks(CHUNK * BATCH) {
        let parts: Vec<String> = batch
            .par_chunks(CHUNK)
            .map(|chunk| {
                let mut s = String::with_capacity(chunk.len() * 48);
                for v in chunk {
                    let _ = writeln!(s, "<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", v[0], v[1], v[2]);
                }
                s
            })
            .collect();
        for p in &parts {
            zip.write_all(p.as_bytes())?;
        }
    }
    zip.write_all(b"</vertices><triangles>\n")?;
    for batch in mesh.triangles.chunks(CHUNK * BATCH) {
        let parts: Vec<String> = batch
            .par_chunks(CHUNK)
            .map(|chunk| {
                let mut s = String::with_capacity(chunk.len() * 44);
                for t in chunk {
                    let _ = writeln!(s, "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>", t[0], t[1], t[2]);
                }
                s
            })
            .collect();
        for p in &parts {
            zip.write_all(p.as_bytes())?;
        }
    }
    zip.write_all(MODEL_TAIL.as_bytes())?;
    zip.finish()?.flush()?;
    Ok(())
}

pub fn read(path: &Path) -> Result<TriMesh> {
    read_from(std::io::BufReader::new(std::fs::File::open(path)?))
}

/// Reads a 3MF package from any seekable reader.
pub fn read_from<R: Read + Seek>(reader: R) -> Result<TriMesh> {
    let mut archive = ZipArchive::new(reader).context("not a valid 3MF (zip) archive")?;
    let mut mesh = TriMesh::default();
    let mut parts = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if !entry.name().to_ascii_lowercase().ends_with(".model") {
            continue;
        }
        let mut xml = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut xml)?;
        parse_model(&xml, &mut mesh)?;
        parts += 1;
    }
    if parts == 0 {
        bail!("3MF archive has no .model part");
    }
    // 3MF vertices are already indexed, but different objects may duplicate
    // coordinates; re-weld so topology checks see one surface.
    let soup: Vec<[[f32; 3]; 3]> = mesh
        .triangles
        .iter()
        .map(|t| [mesh.vertices[t[0] as usize], mesh.vertices[t[1] as usize], mesh.vertices[t[2] as usize]])
        .collect();
    Ok(TriMesh::from_soup(&soup))
}

/// Scans one model part for `<vertex>` and `<triangle>` elements. Triangle
/// indices are relative to the enclosing `<mesh>`.
fn parse_model(xml: &[u8], mesh: &mut TriMesh) -> Result<()> {
    let mut base = mesh.vertices.len() as u32;
    let mut pos = 0usize;
    while let Some(off) = find_byte(&xml[pos..], b'<') {
        let start = pos + off + 1;
        let Some(len) = find_byte(&xml[start..], b'>') else { break };
        let tag = &xml[start..start + len];
        pos = start + len + 1;
        let name_end = tag.iter().position(|b| b.is_ascii_whitespace() || *b == b'/').unwrap_or(tag.len());
        // Drop any namespace prefix ("m:vertex").
        let name = &tag[..name_end];
        let name = match name.iter().rposition(|&b| b == b':') {
            Some(c) => &name[c + 1..],
            None => name,
        };
        match name {
            b"mesh" => base = mesh.vertices.len() as u32,
            b"vertex" => {
                let x = attr_f32(tag, b"x")?;
                let y = attr_f32(tag, b"y")?;
                let z = attr_f32(tag, b"z")?;
                mesh.vertices.push([x, y, z]);
            }
            b"triangle" => {
                let v1 = attr_u32(tag, b"v1")?;
                let v2 = attr_u32(tag, b"v2")?;
                let v3 = attr_u32(tag, b"v3")?;
                mesh.triangles.push([base + v1, base + v2, base + v3]);
            }
            _ => {}
        }
    }
    let n = mesh.vertices.len() as u32;
    if mesh.triangles.iter().any(|t| t.iter().any(|&v| v >= n)) {
        bail!("3MF triangle references a vertex that does not exist");
    }
    Ok(())
}

fn find_byte(haystack: &[u8], needle: u8) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
}

/// Value of attribute `name` inside a tag body such as `vertex x="1" y="2"`.
fn attr<'a>(tag: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    let mut i = 0usize;
    while i + name.len() + 1 < tag.len() {
        let boundary = i == 0 || tag[i - 1].is_ascii_whitespace();
        if boundary && tag[i..].starts_with(name) {
            let mut j = i + name.len();
            while j < tag.len() && tag[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < tag.len() && tag[j] == b'=' {
                j += 1;
                while j < tag.len() && tag[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < tag.len() && (tag[j] == b'"' || tag[j] == b'\'') {
                    let quote = tag[j];
                    let rest = &tag[j + 1..];
                    return rest.iter().position(|&b| b == quote).map(|end| &rest[..end]);
                }
            }
        }
        i += 1;
    }
    None
}

fn attr_f32(tag: &[u8], name: &[u8]) -> Result<f32> {
    let raw = attr(tag, name).context("3MF vertex is missing a coordinate")?;
    std::str::from_utf8(raw)?.trim().parse::<f32>().context("bad 3MF vertex coordinate")
}

fn attr_u32(tag: &[u8], name: &[u8]) -> Result<u32> {
    let raw = attr(tag, name).context("3MF triangle is missing an index")?;
    std::str::from_utf8(raw)?.trim().parse::<u32>().context("bad 3MF triangle index")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::box_mesh;

    #[test]
    fn round_trip_preserves_geometry() {
        let dir = std::env::temp_dir().join(format!("bone-infill-3mf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("box.3mf");
        let m = box_mesh([0.1, 0.2, 0.3], [1.7, 2.9, 3.3]);
        write(&path, &m).unwrap();
        let back = read(&path).unwrap();
        assert_eq!(back.vertices.len(), 8);
        assert_eq!(back.triangles.len(), 12);
        assert!((back.signed_volume() - m.signed_volume()).abs() < 1e-9);
        assert!(back.edge_report().is_watertight());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn attributes_are_parsed_regardless_of_order_and_spacing() {
        let tag = br#"vertex  y = "2.5" x="1" z='-3e-1' "#;
        assert_eq!(attr_f32(tag, b"x").unwrap(), 1.0);
        assert_eq!(attr_f32(tag, b"y").unwrap(), 2.5);
        assert_eq!(attr_f32(tag, b"z").unwrap(), -0.3);
        assert!(attr(tag, b"w").is_none());
    }

    #[test]
    fn multiple_meshes_are_offset() {
        let xml = br#"<model><resources>
            <object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices>
            <triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>
            <object id="2"><mesh><vertices><vertex x="5" y="0" z="0"/><vertex x="6" y="0" z="0"/><vertex x="5" y="1" z="0"/></vertices>
            <triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>
            </resources></model>"#;
        let mut mesh = TriMesh::default();
        parse_model(xml, &mut mesh).unwrap();
        assert_eq!(mesh.triangles, vec![[0, 1, 2], [3, 4, 5]]);
    }
}
