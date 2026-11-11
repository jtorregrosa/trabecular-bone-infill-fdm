//! The web app's API, independent of how requests arrive.
//!
//! [`respond`] takes a method, a URL and a body and returns a [`Reply`]. The
//! native server ([`crate::serve`]) feeds it HTTP requests; the WebAssembly
//! build calls it directly from a Web Worker. Both therefore behave the same.
//!
//! | Request | Does |
//! |---|---|
//! | `GET /` | the app page |
//! | `GET /api/schema` | the parameter form ([`crate::schema`]) |
//! | `GET /api/defaults` | default [`Params`] as JSON |
//! | `GET /api/state` | what is currently loaded |
//! | `POST /api/model?name=..` | body is an STL or 3MF file; becomes the model |
//! | `POST /api/generate` | body `{ "params": {..}, "layer_height": 0.2 }`; runs the pipeline and the check |
//! | `GET /api/mesh/model`, `/api/mesh/result` | geometry for the viewer ([`mesh_binary`]) |
//! | `GET /api/download?format=3mf` | the result as a file (`3mf` or `stl`) |
//!
//! One model and one result are kept in memory.

use crate::analyze::{AnalyzeParams, analyze};
use crate::mesh::TriMesh;
use crate::params::Params;
use crate::util::now_seconds;
use crate::viewer::mesh_binary;
use crate::{io, pipeline};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The app page: the React app in `ui/apps/bone-infill`, built into one
/// self-contained file. It is committed, so building the crate needs no
/// Node.js; rebuild it with `npm run build` in `ui/` after changing the UI.
///
/// Only the native server serves it: the WebAssembly build hosts the page as
/// a static file, so the module does not carry a copy.
#[cfg(feature = "cli")]
const APP_HTML: &str = include_str!("web/ui/index.html");

/// The sample the page offers ("Try a sample"), made by
/// `examples/sample_model.rs`. Served apart from the page, which fetches it
/// only when it is asked for.
#[cfg(feature = "cli")]
const SAMPLE_STL: &[u8] = include_bytes!("web/ui/sample.stl");

struct Model {
    name: String,
    mesh: TriMesh,
}

/// What the app holds between requests.
#[derive(Default)]
pub struct Shared {
    model: Mutex<Option<Arc<Model>>>,
    result: Mutex<Option<Arc<TriMesh>>>,
    /// Held while generating: one run at a time.
    busy: Mutex<()>,
    /// Upper bound on the voxel budget a request may ask for. Used where
    /// memory is scarce, such as in a browser.
    max_voxels: Option<f64>,
}

impl Shared {
    /// State for an environment that cannot afford more than `max_voxels`.
    pub fn with_voxel_limit(max_voxels: f64) -> Self {
        Self { max_voxels: Some(max_voxels), ..Self::default() }
    }
}

/// A reply, independent of the HTTP library.
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    /// Sent as an attachment with this file name.
    pub download: Option<String>,
}

impl Reply {
    fn json(value: serde_json::Value) -> Self {
        Self { status: 200, content_type: "application/json", body: value.to_string().into_bytes(), download: None }
    }
    pub fn error(status: u16, message: impl std::fmt::Display) -> Self {
        Self { status, ..Self::json(json!({ "error": message.to_string() })) }
    }
    fn bytes(content_type: &'static str, body: Vec<u8>) -> Self {
        Self { status: 200, content_type, body, download: None }
    }
}

#[derive(Deserialize)]
struct GenerateRequest {
    #[serde(default)]
    params: Params,
    layer_height: Option<f64>,
}

/// The app page.
#[cfg(feature = "cli")]
pub fn page() -> &'static str {
    APP_HTML
}

/// Decodes `%XX` escapes and `+` in a query value.
fn percent_decode(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) =
                (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == key).then(|| percent_decode(v))
    })
}

/// A file name with no directory part and nothing that could break a header.
pub(crate) fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base.chars().filter(|c| !c.is_control() && !matches!(c, '"' | ';')).collect();
    if clean.trim().is_empty() { "model.stl".to_string() } else { clean }
}

fn extension_of(name: &str) -> String {
    Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

fn load_model(shared: &Shared, name: &str, body: &[u8]) -> Result<serde_json::Value> {
    let extension = extension_of(name);
    if extension != "stl" && extension != "3mf" {
        bail!("'{name}' is not an STL or 3MF file");
    }
    if body.is_empty() {
        bail!("the file is empty");
    }
    let (mesh, notes) = pipeline::prepare_model(io::read_mesh_bytes(name, body)?)?;
    let (lo, hi) = mesh.bounds();
    let info = json!({
        "name": name,
        "size": [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
        "volume": mesh.signed_volume(),
        "triangles": mesh.triangles.len(),
        "notes": notes,
    });
    println!("model: {name} ({} triangles)", mesh.triangles.len());
    *shared.model.lock().unwrap() = Some(Arc::new(Model { name: name.to_string(), mesh }));
    *shared.result.lock().unwrap() = None;
    Ok(info)
}

fn generate(shared: &Shared, body: &[u8]) -> Result<serde_json::Value> {
    let mut request: GenerateRequest = serde_json::from_slice(body).context("invalid parameters")?;
    if let Some(limit) = shared.max_voxels {
        request.params.max_voxels = request.params.max_voxels.min(limit);
    }
    request.params.validate()?;
    let layer_height = request.layer_height.unwrap_or(AnalyzeParams::default().layer_height);
    if !(0.02..=2.0).contains(&layer_height) {
        bail!("layer height must be between 0.02 and 2 mm (got {layer_height})");
    }
    let model = shared.model.lock().unwrap().clone().context("load a model first")?;

    let _running = shared.busy.lock().unwrap();
    let started = now_seconds();
    let generated = pipeline::generate(&model.mesh, &request.params, &|_| {})?;
    let check = AnalyzeParams {
        layer_height,
        max_overhang_deg: request.params.max_overhang_deg,
        max_bridge: request.params.max_bridge,
        min_bridge_width: request.params.min_bridge_width,
        skin: Some(request.params.shell),
        ..AnalyzeParams::default()
    };
    let printability = analyze(&generated.mesh, Some(&model.mesh), &check);
    let seconds = now_seconds() - started;
    println!(
        "generated {}: {} triangles, {:.1} mm2 unsupported inside, {seconds:.2} s",
        model.name,
        generated.mesh.triangles.len(),
        printability.unsupported_interior_area
    );
    let reply = json!({ "report": generated.report, "printability": printability, "seconds": seconds });
    *shared.result.lock().unwrap() = Some(Arc::new(generated.mesh));
    Ok(reply)
}

fn download(shared: &Shared, format: &str) -> Result<Reply> {
    let (extension, content_type) = match format {
        "3mf" => ("3mf", "model/3mf"),
        "stl" => ("stl", "model/stl"),
        other => bail!("unknown format '{other}' (use 3mf or stl)"),
    };
    let mesh = shared.result.lock().unwrap().clone().context("nothing has been generated yet")?;
    let stem = shared
        .model
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| Path::new(&m.name).file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .unwrap_or_else(|| "model".to_string());
    let body = io::write_mesh_bytes(extension, &mesh)?;
    Ok(Reply { download: Some(format!("{stem}_bone.{extension}")), ..Reply::bytes(content_type, body) })
}

/// Answers one request. `method` is upper case, `url` is the path with its
/// query string.
pub fn respond(shared: &Shared, method: &str, url: &str, body: &[u8]) -> Reply {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let fallible = |result: Result<serde_json::Value>| match result {
        Ok(value) => Reply::json(value),
        Err(e) => Reply::error(400, format!("{e:#}")),
    };
    match (method, path) {
        #[cfg(feature = "cli")]
        ("GET", "/") | ("GET", "/index.html") => Reply::bytes("text/html; charset=utf-8", page().as_bytes().to_vec()),
        #[cfg(feature = "cli")]
        ("GET", "/sample.stl") => Reply::bytes("model/stl", SAMPLE_STL.to_vec()),
        ("GET", "/api/defaults") => Reply::json(json!(Params::default())),
        ("GET", "/api/schema") => Reply::json(crate::schema::schema()),
        ("GET", "/api/state") => {
            let model = shared.model.lock().unwrap().as_ref().map(|m| json!({ "name": m.name }));
            Reply::json(json!({ "model": model, "result": shared.result.lock().unwrap().is_some() }))
        }
        ("POST", "/api/model") => {
            let name = safe_name(&query_value(query, "name").unwrap_or_default());
            fallible(load_model(shared, &name, body))
        }
        ("POST", "/api/generate") => fallible(generate(shared, body)),
        ("GET", "/api/mesh/model") => match shared.model.lock().unwrap().clone() {
            Some(model) => Reply::bytes("application/octet-stream", mesh_binary(&model.mesh)),
            None => Reply::error(404, "no model loaded"),
        },
        ("GET", "/api/mesh/result") => match shared.result.lock().unwrap().clone() {
            Some(mesh) => Reply::bytes("application/octet-stream", mesh_binary(&mesh)),
            None => Reply::error(404, "nothing has been generated yet"),
        },
        ("GET", "/api/download") => {
            download(shared, &query_value(query, "format").unwrap_or_else(|| "3mf".to_string()))
                .unwrap_or_else(|e| Reply::error(400, format!("{e:#}")))
        }
        ("GET", _) | ("POST", _) => Reply::error(404, "not found"),
        _ => Reply::error(405, "method not allowed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::stl;
    use crate::mesh::box_mesh;

    fn stl_bytes(mesh: &TriMesh) -> Vec<u8> {
        io::write_mesh_bytes("stl", mesh).unwrap()
    }

    #[test]
    fn voxel_limit_caps_what_a_request_can_ask_for() {
        let shared = Shared::with_voxel_limit(1.0e6);
        respond(&shared, "POST", "/api/model?name=a.stl", &stl_bytes(&box_mesh([0.0; 3], [40.0; 3])));
        let out = respond(&shared, "POST", "/api/generate", br#"{"params": {"cell": 12, "waves": 16}}"#);
        assert_eq!(out.status, 200, "{}", String::from_utf8_lossy(&out.body));
        let report = json_of(&out);
        assert_eq!(report["report"]["params"]["max_voxels"], 1.0e6);
        assert!(report["report"]["voxels"].as_u64().unwrap() <= 1_100_000);
        assert!(report["report"]["voxel"].as_f64().unwrap() > 0.3);
    }

    fn json_of(reply: &Reply) -> serde_json::Value {
        serde_json::from_slice(&reply.body).unwrap()
    }

    #[test]
    fn page_and_defaults_are_served() {
        let shared = Shared::default();
        let page = respond(&shared, "GET", "/", b"");
        assert_eq!(page.status, 200);
        let html = String::from_utf8(page.body).unwrap();
        assert!(html.contains("function createViewer"), "viewer core was not inlined");
        assert!(html.contains(r#"<div id="root">"#));
        // Self-contained: nothing is loaded from elsewhere.
        assert!(!html.contains(r#"src="http"#) && !html.contains(r#"href="http"#));
        let schema = json_of(&respond(&shared, "GET", "/api/schema", b""));
        assert_eq!(schema["product"]["name"], "bone-infill");
        // The sample is a closed mesh the app accepts as it is.
        let sample = respond(&shared, "GET", "/sample.stl", b"");
        assert_eq!(sample.status, 200);
        let mesh = io::read_mesh_bytes("sample.stl", &sample.body).unwrap();
        let edges = mesh.edge_report();
        assert_eq!(edges.boundary + edges.non_manifold + edges.misoriented, 0);
        let defaults = json_of(&respond(&shared, "GET", "/api/defaults", b""));
        assert_eq!(defaults["cell"], 6.0);
        assert_eq!(defaults["open"], json!([]));
    }

    #[test]
    fn whole_session_upload_generate_download() {
        let shared = Shared::default();
        // Nothing loaded yet.
        assert_eq!(respond(&shared, "GET", "/api/mesh/model", b"").status, 404);
        assert_eq!(respond(&shared, "POST", "/api/generate", b"{}").status, 400);
        assert_eq!(respond(&shared, "GET", "/api/download?format=3mf", b"").status, 400);

        let model = stl_bytes(&box_mesh([0.0; 3], [24.0; 3]));
        let info = respond(&shared, "POST", "/api/model?name=my%20cube.stl", &model);
        assert_eq!(info.status, 200, "{}", String::from_utf8_lossy(&info.body));
        let info = json_of(&info);
        assert_eq!(info["name"], "my cube.stl");
        assert_eq!(info["triangles"], 12);
        assert_eq!(json_of(&respond(&shared, "GET", "/api/state", b""))["model"]["name"], "my cube.stl");
        assert_eq!(&respond(&shared, "GET", "/api/mesh/model", b"").body[..4], b"BIM1");

        let request = br#"{"params": {"voxel": 0.5, "cell": 10, "shell": 1.5, "waves": 32, "open": ["+z"]}, "layer_height": 0.2}"#;
        let out = respond(&shared, "POST", "/api/generate", request);
        assert_eq!(out.status, 200, "{}", String::from_utf8_lossy(&out.body));
        let out = json_of(&out);
        assert_eq!(out["report"]["params"]["open"], json!(["+z"]));
        assert!(out["report"]["triangles"].as_u64().unwrap() > 100);
        assert!(out["printability"]["layers"].as_u64().unwrap() > 100);

        assert_eq!(&respond(&shared, "GET", "/api/mesh/result", b"").body[..4], b"BIM1");
        let file = respond(&shared, "GET", "/api/download?format=stl", b"");
        assert_eq!(file.status, 200);
        assert_eq!(file.download.as_deref(), Some("my cube_bone.stl"));
        assert!(stl::parse(&file.body).unwrap().len() > 100);
        let file = respond(&shared, "GET", "/api/download?format=3mf", b"");
        assert_eq!(&file.body[..2], b"PK");
    }

    #[test]
    fn bad_requests_get_readable_errors() {
        let shared = Shared::default();
        let reply = respond(&shared, "POST", "/api/model?name=notes.txt", b"hello");
        assert_eq!(reply.status, 400);
        assert!(json_of(&reply)["error"].as_str().unwrap().contains("not an STL or 3MF"));
        assert_eq!(respond(&shared, "POST", "/api/model?name=a.stl", b"garbage").status, 400);

        let model = stl_bytes(&box_mesh([0.0; 3], [24.0; 3]));
        respond(&shared, "POST", "/api/model?name=a.stl", &model);
        let reply = respond(&shared, "POST", "/api/generate", br#"{"params": {"density": 3}}"#);
        assert_eq!(reply.status, 400);
        assert!(json_of(&reply)["error"].as_str().unwrap().contains("density"));
        assert_eq!(respond(&shared, "GET", "/nope", b"").status, 404);
        assert_eq!(respond(&shared, "DELETE", "/", b"").status, 405);
        assert_eq!(respond(&shared, "GET", "/api/download?format=obj", b"").status, 400);
    }

    #[test]
    fn names_are_decoded_and_sanitised() {
        assert_eq!(percent_decode("a%20b+c%2Fd"), "a b c/d");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(safe_name("../../etc/passwd.stl"), "passwd.stl");
        assert_eq!(safe_name("C:\\tmp\\a\"b;.stl"), "ab.stl");
        assert_eq!(safe_name(""), "model.stl");
        assert_eq!(query_value("format=stl&name=x", "name").as_deref(), Some("x"));
        assert_eq!(query_value("format=stl", "name"), None);
    }
}
