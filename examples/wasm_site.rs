//! Builds the WebAssembly version of the web app into `wasm/dist/`.
//!
//! ```text
//! rustup target add wasm32-unknown-unknown      (once)
//! cargo run --release --example wasm_site
//! ```
//!
//! The result is three static files (`index.html`, `worker.js`,
//! `bone_infill_wasm.wasm`) that any static web server can host. Browsers do
//! not start workers from `file://`, so to try it locally serve the folder:
//!
//! ```text
//! python -m http.server 8080 --directory wasm/dist
//! ```
//!
//! `index.html` is the very page the native server serves, plus `shim.js`,
//! which answers its API calls from the worker instead of over HTTP.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let wasm = root.join("wasm");
    let dist = wasm.join("dist");

    // The crate's own .cargo/config.toml selects the wasm target.
    let status = Command::new(env!("CARGO"))
        .args(["build", "--release"])
        .current_dir(&wasm)
        .status()
        .context("running cargo for the wasm crate")?;
    if !status.success() {
        bail!("the wasm crate did not build (is the target installed? rustup target add wasm32-unknown-unknown)");
    }

    std::fs::create_dir_all(&dist)?;
    let module = wasm.join("target/wasm32-unknown-unknown/release/bone_infill_wasm.wasm");
    let size = std::fs::copy(&module, dist.join("bone_infill_wasm.wasm"))
        .with_context(|| format!("copying {}", module.display()))?;
    std::fs::copy(wasm.join("web/worker.js"), dist.join("worker.js"))?;
    // The sample model, fetched by the page only when asked for.
    std::fs::copy(root.join("src/web/ui/sample.stl"), dist.join("sample.stl"))?;
    // Icons for browsers and devices that do not take the page's inline SVG,
    // and the preview image for shared links (docs/brand).
    let brand = root.join("docs/brand");
    for (from, to) in [
        ("favicon.ico", "favicon.ico"),
        ("png/icon-180.png", "apple-touch-icon.png"),
        ("png/social-card.png", "social-card.png"),
    ] {
        std::fs::copy(brand.join(from), dist.join(to)).with_context(|| format!("copying docs/brand/{from}"))?;
    }

    // The app page, with the shim first in <head>: a classic script runs
    // before the app's module script, so the API calls are routed from the start.
    let page = bone_infill::app::page();
    let shim = std::fs::read_to_string(wasm.join("web/shim.js"))?;
    let at = page.find("<head>").context("the app page has no <head> to hook into")? + "<head>".len();
    let html = format!("{}\n<script>\n{}</script>\n{}{}", &page[..at], shim, site_head(), &page[at..]);
    std::fs::write(dist.join("index.html"), html)?;

    println!("{} ({:.1} MB of WebAssembly)", dist.display(), size as f64 / 1048576.0);
    println!("serve it with: python -m http.server 8080 --directory {}", dist.display());
    Ok(())
}

/// Tags only the hosted site needs: device icons and the link preview.
/// Previews need absolute URLs, so they are added only when `SITE_URL` (the
/// address the site will live at) is set, as the GitHub Pages workflow does.
fn site_head() -> String {
    let mut head = String::from(
        "<link rel=\"icon\" href=\"favicon.ico\" sizes=\"48x48\">\n\
         <link rel=\"apple-touch-icon\" href=\"apple-touch-icon.png\">\n",
    );
    if let Ok(url) = std::env::var("SITE_URL") {
        let url = url.trim_end_matches('/');
        let title = bone_infill::schema::PRODUCT_NAME;
        let text = "Fill a 3D-printable part with a trabecular-bone lattice that prints without supports. Runs in your browser.";
        head += &format!(
            "<meta property=\"og:type\" content=\"website\">\n\
             <meta property=\"og:title\" content=\"{title}\">\n\
             <meta property=\"og:description\" content=\"{text}\">\n\
             <meta property=\"og:url\" content=\"{url}/\">\n\
             <meta property=\"og:image\" content=\"{url}/social-card.png\">\n\
             <meta property=\"og:image:width\" content=\"1200\">\n\
             <meta property=\"og:image:height\" content=\"630\">\n\
             <meta name=\"twitter:card\" content=\"summary_large_image\">\n"
        );
    }
    head
}
