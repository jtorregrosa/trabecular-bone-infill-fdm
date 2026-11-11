//! Interactive viewer: writes a mesh as one self-contained HTML page.
//!
//! The page embeds the geometry and a small WebGL 2 renderer, so it opens
//! from disk in any current browser with no server and no network access. It
//! offers orbit / pan / zoom, a section plane along X, Y or Z (the cut faces
//! are shaded as solid material, as in the PNG preview), and a colour mode
//! that paints down-facing surfaces by their overhang angle.
//!
//! Vertex positions are quantised to 16 bits per axis over the bounding box
//! (3 µm on a 200 mm part) and normals are derived in the fragment shader, so
//! the page is about 8 bytes per vertex and 16 per triangle after base64.

use crate::mesh::TriMesh;
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use std::io::Write;
use std::path::Path;

const TEMPLATE: &str = include_str!("web/viewer.html");
/// WebGL renderer and plain-HTML view controls. It lives in the studio kit
/// (`ui/`), whose React viewer wraps the same file.
pub const VIEWER_CORE_JS: &str = include_str!("../ui/packages/ui/src/studio/viewer-core.js");
/// Slot in a page template where [`VIEWER_CORE_JS`] is inserted.
pub const CORE_SLOT: &str = "/*{{VIEWER_CORE}}*/";
/// Design tokens and shared control styles (see `docs/DESIGN.md`).
pub const THEME_CSS: &str = include_str!("web/theme.css");
/// Slot in a page's `<style>` where [`THEME_CSS`] is inserted.
pub const THEME_SLOT: &str = "/*{{THEME}}*/";

/// The app icon from the brand pack (`docs/brand`).
pub const ICON_SVG: &str = include_str!("../docs/brand/svg/icon.svg");
/// Slot in a page template for the icon, as a data URI.
pub const ICON_SLOT: &str = "{{ICON}}";

/// Fills the slots every page template shares.
pub fn with_shared_assets(template: &str) -> String {
    let icon = format!("data:image/svg+xml;base64,{}", String::from_utf8(base64(ICON_SVG.as_bytes())).unwrap());
    template.replace(THEME_SLOT, THEME_CSS).replace(CORE_SLOT, VIEWER_CORE_JS).replace(ICON_SLOT, &icon)
}

/// What the page shows besides the model.
#[derive(Clone, Debug, Default)]
pub struct ViewerInfo {
    pub title: String,
    /// Lets the Z section report the printed layer it is on.
    pub layer_height: Option<f64>,
    /// `(label, value)` rows of the report table.
    pub rows: Vec<(String, String)>,
}

#[derive(serde::Serialize)]
struct Meta<'a> {
    lo: [f32; 3],
    hi: [f32; 3],
    vertices: usize,
    triangles: usize,
    layer_height: Option<f64>,
    info: &'a [(String, String)],
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = vec![b'='; bytes.len().div_ceil(3) * 4];
    out.par_chunks_mut(4).zip(bytes.par_chunks(3)).for_each(|(dst, src)| {
        let b = [src[0], *src.get(1).unwrap_or(&0), *src.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        dst[0] = ALPHABET[(n >> 18) as usize & 63];
        dst[1] = ALPHABET[(n >> 12) as usize & 63];
        if src.len() > 1 {
            dst[2] = ALPHABET[(n >> 6) as usize & 63];
        }
        if src.len() > 2 {
            dst[3] = ALPHABET[n as usize & 63];
        }
    });
    out
}

/// Geometry in the compact form the pages draw from.
struct Packed {
    lo: [f32; 3],
    hi: [f32; 3],
    /// Little-endian `u16` triples, quantised over the bounding box.
    positions: Vec<u8>,
    /// Little-endian `u32` triples.
    indices: Vec<u8>,
}

fn pack(mesh: &TriMesh) -> Packed {
    let (lo, hi) = mesh.bounds();
    let lo = lo.map(|v| v as f32);
    let hi = hi.map(|v| v as f32);
    let scale: [f32; 3] = std::array::from_fn(|a| if hi[a] > lo[a] { 65535.0 / (hi[a] - lo[a]) } else { 0.0 });
    let positions: Vec<u8> = mesh
        .vertices
        .par_iter()
        .flat_map_iter(|v| {
            let q: [u16; 3] = std::array::from_fn(|a| ((v[a] - lo[a]) * scale[a]).round().clamp(0.0, 65535.0) as u16);
            let mut bytes = [0u8; 6];
            for a in 0..3 {
                bytes[a * 2..a * 2 + 2].copy_from_slice(&q[a].to_le_bytes());
            }
            bytes
        })
        .collect();
    let indices: Vec<u8> =
        mesh.triangles.par_iter().flat_map_iter(|t| t.iter().flat_map(|v| v.to_le_bytes())).collect();
    Packed { lo, hi, positions, indices }
}

/// The mesh as one binary blob for the web app:
/// `"BIM1"`, vertex count, triangle count (`u32`), bounding box (`6 x f32`),
/// quantised positions (`u16` triples, padded to 4 bytes), indices (`u32`
/// triples). Everything little-endian.
pub fn mesh_binary(mesh: &TriMesh) -> Vec<u8> {
    let packed = pack(mesh);
    let mut out = Vec::with_capacity(40 + packed.positions.len() + packed.indices.len());
    out.extend_from_slice(b"BIM1");
    out.extend_from_slice(&(mesh.vertices.len() as u32).to_le_bytes());
    out.extend_from_slice(&(mesh.triangles.len() as u32).to_le_bytes());
    for v in packed.lo.iter().chain(&packed.hi) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&packed.positions);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out.extend_from_slice(&packed.indices);
    out
}

/// Writes `mesh` as an interactive HTML page.
pub fn write_viewer(mesh: &TriMesh, path: &Path, info: &ViewerInfo) -> Result<()> {
    if mesh.is_empty() {
        bail!("nothing to show: the mesh has no triangles");
    }
    let Packed { lo, hi, positions, indices } = pack(mesh);

    let meta = Meta {
        lo,
        hi,
        vertices: mesh.vertices.len(),
        triangles: mesh.triangles.len(),
        layer_height: info.layer_height,
        info: &info.rows,
    };
    // "</" must not appear inside a <script> element.
    let meta_json = serde_json::to_string(&meta)?.replace("</", "<\\/");
    let page =
        with_shared_assets(TEMPLATE).replace("{{TITLE}}", &escape_html(&info.title)).replace("{{META}}", &meta_json);
    let (head, rest) = page.split_once("{{POSITIONS}}").context("viewer template is missing its position slot")?;
    let (middle, tail) = rest.split_once("{{INDICES}}").context("viewer template is missing its index slot")?;

    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let file = std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
    out.write_all(head.as_bytes())?;
    out.write_all(&base64(&positions))?;
    out.write_all(middle.as_bytes())?;
    out.write_all(&base64(&indices))?;
    out.write_all(tail.as_bytes())?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::box_mesh;

    #[test]
    fn base64_matches_reference_vectors() {
        // RFC 4648 section 10.
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected.as_bytes(), "{input}");
        }
    }

    #[test]
    fn page_embeds_the_mesh_and_escapes_the_title() {
        let dir = std::env::temp_dir().join(format!("bone-infill-viewer-{}", std::process::id()));
        let path = dir.join("box.html");
        let mesh = box_mesh([0.0; 3], [10.0, 20.0, 30.0]);
        let info = ViewerInfo {
            title: "a<b>&\"c\"".into(),
            layer_height: Some(0.2),
            rows: vec![("volume".into(), "6.0 cm3".into()), ("note".into(), "</script>".into())],
        };
        write_viewer(&mesh, &path, &info).unwrap();
        let page = std::fs::read_to_string(&path).unwrap();
        assert!(page.contains("a&lt;b&gt;&amp;&quot;c&quot;"));
        assert!(!page.contains("{{"), "unfilled template slot");
        assert!(page.contains("rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml;base64,PHN2Zy"));
        // The only closing script tags are the template's own four.
        assert_eq!(page.matches("</script>").count(), 4);
        assert!(page.contains("\"triangles\":12"));
        // 8 vertices * 6 bytes = 48 bytes -> 64 base64 characters, starting
        // with vertex (0, 0, 0) and containing a full-scale coordinate.
        let start = page.find("id=\"positions\"").unwrap();
        let data = &page[page[start..].find('>').unwrap() + start + 1..];
        let data = &data[..data.find('<').unwrap()];
        assert_eq!(data.len(), 64);
        assert!(data.starts_with("AAAAAAAA"));
        assert!(data.contains("//"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn binary_blob_has_the_documented_layout() {
        let mesh = box_mesh([0.0; 3], [10.0, 20.0, 30.0]);
        let blob = mesh_binary(&mesh);
        assert_eq!(&blob[..4], b"BIM1");
        assert_eq!(u32::from_le_bytes(blob[4..8].try_into().unwrap()), 8);
        assert_eq!(u32::from_le_bytes(blob[8..12].try_into().unwrap()), 12);
        assert_eq!(f32::from_le_bytes(blob[32..36].try_into().unwrap()), 30.0);
        // 36-byte header + 8 * 6 position bytes (already a multiple of 4) + 12 * 12 index bytes.
        assert_eq!(blob.len(), 36 + 48 + 144);
        // An odd vertex count needs two bytes of padding before the indices.
        let mut odd = mesh.clone();
        odd.vertices.push([1.0, 1.0, 1.0]);
        assert_eq!(mesh_binary(&odd).len(), 36 + 54 + 2 + 144);
    }

    #[test]
    fn empty_mesh_is_rejected() {
        let path = std::env::temp_dir().join("bone-infill-empty.html");
        assert!(write_viewer(&TriMesh::default(), &path, &ViewerInfo::default()).is_err());
    }
}
