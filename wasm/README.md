# bone-infill in the browser (WebAssembly port)

The same web app as `bone-infill serve`, with no server: the algorithm is compiled to
WebAssembly and runs in a Web Worker in the page. Three static files, hostable anywhere.

**Status: working prototype.** The native program in the repository root is the reference
implementation; this port reuses its code unchanged and adds only glue.

## Build and run

```bash
rustup target add wasm32-unknown-unknown
```

```bash
cargo run --release --example wasm_site
```

```bash
python -m http.server 8080 --directory wasm/dist
```

Then open `http://localhost:8080/`. Run the first two from the repository root. The second
builds this crate and assembles `wasm/dist/`: `index.html`, `worker.js`,
`bone_infill_wasm.wasm`, the sample model, and the icons and link preview from
[the brand pack](../docs/brand/README.md). Browsers do not start workers from `file://`, hence
the local server; any static host works the same way, at the root or in a subfolder (every
path is relative).

## Publish on GitHub Pages

`.github/workflows/pages.yml` builds the site and publishes it on every push to `main`, or by
hand from the Actions tab. Once per repository:

1. Push the repository to GitHub (Pages on a private repository needs a paid plan).
2. In **Settings > Pages**, set **Source** to **GitHub Actions**.
3. Push to `main`, or run the **Pages** workflow. The address appears in the workflow run and
   in Settings > Pages: `https://<user>.github.io/<repository>/`.

The workflow needs only Rust and the wasm32 target: the app page is built from `ui/` and
committed, so after a change to the interface run `npm run build` in `ui/` and commit
`src/web/ui/index.html` before pushing. It sets `SITE_URL` to the Pages address, which adds
the tags for link previews (title, description and `social-card.png` on MakerWorld, social
networks and chats); those need absolute URLs, so a local build leaves them out.

## How it fits together

```
page (src/web/ui/index.html, unchanged)
  │  fetch("api/...")
  ▼
shim.js ── postMessage ──► worker.js ──► request() in the .wasm
                                            │
                                            ▼
                              bone_infill::app::respond   (the same function the
                                                           native HTTP server calls)
```

- `src/lib.rs` exports a small C ABI (`alloc`, `request`, `reply_*`) over the module's
  memory. No `wasm-bindgen`: the only tool needed is the Rust target.
- `web/worker.js` loads the module and turns messages into calls.
- `web/shim.js` is added to the page at build time. It answers the page's API calls from
  the worker and turns the download links into saved files.

Because both front ends go through `app::respond`, a change to the algorithm or to the page
shows up in both without touching this folder.

## What to expect

Measured in Chromium on a Ryzen 7 5800X, same parameters as the native runs:

| Model | Native (16 threads) | Browser (1 thread) |
|---|---|---|
| Cube 20 mm | 0.3 s | 2.4 s |
| Stanford bunny 80 mm | about 3 s | 12.4 s |

Results are the same as native: the cube gives the identical mesh (10 588 triangles).

Differences from the native app:

- **One thread.** Rayon runs sequentially without wasm threads. Enabling them is possible
  (nightly toolchain, rebuilt standard library, and a host that sends cross-origin isolation
  headers) but is not set up here.
- **16 million voxels at most** (`MAX_VOXELS` in `src/lib.rs`), against 60 million natively.
  Larger parts get a coarser voxel automatically. The limit keeps memory well under what a
  browser tab can be relied on to have.
- **A failed computation needs a reload.** If the module traps (typically out of memory),
  the worker refuses further work and says so.
- Timings shown per stage come from `performance.now()`.

Not yet done: a progress indicator finer than "Generating…" (the worker is busy until the
whole run ends, so the regeneration cannot be cancelled either), and testing in browsers
other than Chromium.
