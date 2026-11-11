# User interface

The web app's source: an npm workspace with a reusable **studio kit** and the
**bone-infill app** built on it. React, Tailwind CSS v4 and shadcn/ui (Radix), built with Vite.

```
ui/
  packages/ui/              @workspace/ui: the kit, shared by every product
    src/components/         shadcn/ui components, themed (our code, not a dependency)
    src/styles/globals.css  design tokens (docs/DESIGN.md) mapped onto shadcn's roles
    src/studio/             the product shell, with no knowledge of any one generator:
      api.tsx                 client of the studio API, schema types
      use-studio.tsx          session state: schema, parameters, model, result, status
      param-form.tsx          the parameter form, built from GET api/schema
      viewer.tsx              React wrapper of the WebGL renderer
      viewer-core.js          the renderer (also inlined by the Rust crate's `view` page)
      view-controls.tsx       section, shading, colour, camera, keyboard shortcuts
      viewport.tsx            canvas, status toast, empty-state overlay
      file-drop.tsx           drop a file anywhere, or pick one
      info-tip.tsx            explanation behind an info icon (hover, focus or tap)
      preset-picker.tsx       quick choice between the schema's presets
      panel.tsx               panel layout, headings, mark, spinner
  apps/bone-infill/         the app: layout, report and verdict, sample model
```

## Working on it

```bash
npm install
```

```bash
npm run dev
```

```bash
npm run build
```

`dev` serves the app with hot reload and forwards `/api` to `http://127.0.0.1:8791`, so start
`bone-infill serve --port 8791 --no-open` beside it. `build` type-checks and writes one
self-contained file, `../src/web/ui/index.html`, which the Rust crate embeds; commit it with
the source change.

Add a shadcn component to the kit from the app folder:

```bash
npx shadcn@latest add dialog -c apps/bone-infill
```

## A new product on the kit

A generator that answers the same API (see `src/studio/api.tsx`) and describes its parameters
in `GET api/schema` gets the form, the viewer, the status and the downloads as they are. A new
app under `apps/` supplies only what is its own: the report of a result, a sample model, the
layout if it differs. The schema also carries the product name and tagline shown in the header.
The logo is the app's too: bone-infill draws its own from the brand pack (`src/logo.tsx`, see
[the design notes](../docs/DESIGN.md#brand)), and the kit has none.

## Rules

- The page must stay self-contained: no CDN, web font or remote asset. It runs offline, from a
  single binary or from three static files.
- API paths are relative strings (`api/...`), so the page works under any base URL and the
  WebAssembly build can answer them from a worker.
- Download buttons are plain links to `api/download?...`, which that build intercepts.
- Change the view through `viewer.update(...)` and the viewer's methods, not by mutating its
  state.
