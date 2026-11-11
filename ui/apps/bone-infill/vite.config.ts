import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { type Plugin, defineConfig } from "vite"
import { viteSingleFile } from "vite-plugin-singlefile"

// The app builds to one self-contained HTML file that the Rust crate embeds
// (src/web/ui/index.html): no network access, no other files to serve.
//
// `npm run dev` serves it with hot reload and forwards the API to a running
// `bone-infill serve --port 8791`.
const brand = resolve(import.meta.dirname, "../../../docs/brand")

/** Replaces %BRAND:<file>% in index.html with that brand file as a data URI. */
function brandFiles(): Plugin {
  return {
    name: "brand-files",
    transformIndexHtml: (html) =>
      html.replace(/%BRAND:([^%]+)%/g, (_, file: string) => {
        const svg = readFileSync(resolve(brand, file), "utf8").trim()
        return "data:image/svg+xml," + encodeURIComponent(svg)
      }),
  }
}

export default defineConfig({
  base: "./",
  plugins: [
    react(),
    tailwindcss(),
    brandFiles(),
    viteSingleFile({ removeViteModuleLoader: true }),
  ],
  resolve: {
    alias: {
      "@": resolve(import.meta.dirname, "./src"),
      // The brand pack lives with the documentation, outside the workspace.
      "@brand": brand,
    },
  },
  build: {
    outDir: resolve(import.meta.dirname, "../../../src/web/ui"),
    // The folder also holds the sample model (examples/sample_model.rs).
    emptyOutDir: false,
    reportCompressedSize: true,
  },
  server: {
    fs: { allow: [resolve(import.meta.dirname, "../../..")] },
    proxy: {
      "/api": "http://127.0.0.1:8791",
      "/sample.stl": "http://127.0.0.1:8791",
    },
  },
})
