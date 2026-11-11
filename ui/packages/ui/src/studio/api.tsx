// Client of the studio API. Every generator served through this kit answers
// the same routes (see `app::respond` in the Rust crate):
//
//   GET  api/schema             the parameter form and the product name
//   GET  api/defaults           default parameters
//   GET  api/state              what the server already holds
//   POST api/model?name=…       load a mesh (body: the file)
//   POST api/generate           { params, …request fields } → report
//   GET  api/mesh/model|result  meshes in the viewer's binary format
//   GET  api/download?format=…  the result as a file
//
// Paths are relative, so the page works under any base URL, and are plain
// strings, so the WebAssembly build can answer them from a worker.

/** One entry of the parameter form. */
export type Item = {
  key: string
  label: string
  kind?: "number" | "bool" | "seed" | "faces" | "loads" | "vec3" | "optional"
  min?: number
  max?: number
  step?: number
  /** Shown value = stored value × scale. */
  scale?: number
  integer?: boolean
  /** What the parameter is, for its info card. */
  about?: string
  /** What raising or lowering it does, as [label, text] pairs. */
  effects?: [string, string][]
  /** A practical suggestion. */
  tip?: string
  placeholder?: string
  /** A field of the generate request rather than of the parameters. */
  request?: boolean
  /** Default of a request field. */
  default?: unknown
}

export type Group = { title: string; open?: boolean; items: Item[] }

/** Named values applied on top of the current parameters. */
export type Preset = { name: string; about: string; params: Values }

export type Schema = {
  product: { name: string; tagline: string }
  presets?: Preset[]
  groups: Group[]
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type Json = any
export type Values = Record<string, Json>

export type ModelInfo = {
  name: string
  size: [number, number, number]
  volume: number
  triangles: number
  notes: string[]
}

export async function api<T = Json>(
  path: string,
  options?: RequestInit
): Promise<T> {
  const response = await fetch(path, options)
  const body = await response
    .json()
    .catch(() => ({ error: "unexpected reply from the server" }))
  if (!response.ok) throw new Error(body.error || response.statusText)
  return body as T
}

export const getSchema = () => api<Schema>("api/schema")
export const getDefaults = () => api<Values>("api/defaults")
export const getState = () =>
  api<{ model: { name: string } | null; result: boolean }>("api/state")

export const uploadModel = (file: File) =>
  api<ModelInfo>("api/model?name=" + encodeURIComponent(file.name), {
    method: "POST",
    body: file,
  })

export const generate = (params: Values, request: Values) =>
  api("api/generate", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ params, ...request }),
  })

export async function getMesh(which: "model" | "result"): Promise<ArrayBuffer> {
  const response = await fetch("api/mesh/" + which, { cache: "no-store" })
  if (!response.ok) throw new Error("could not fetch the mesh")
  return response.arrayBuffer()
}

export const downloadUrl = (format: string) => `api/download?format=${format}`

// ---- values addressed by schema keys ----------------------------------------

/** Reads the value an item edits; `"a.1"` is element 1 of array `a`. */
export function readValue(item: Item, params: Values, request: Values): Json {
  if (item.request) return request[item.key]
  const [k, i] = item.key.split(".")
  return i === undefined ? params[k] : params[k]?.[+i]
}

/** Returns new `[params, request]` with the item set to `value`. */
export function writeValue(
  item: Item,
  params: Values,
  request: Values,
  value: Json
): [Values, Values] {
  if (item.request) return [params, { ...request, [item.key]: value }]
  const [k, i] = item.key.split(".")
  if (i === undefined) return [{ ...params, [k]: value }, request]
  const array = [...params[k]]
  array[+i] = value
  return [{ ...params, [k]: array }, request]
}

export function sameValue(a: Json, b: Json): boolean {
  if (typeof a === "number" && typeof b === "number")
    return Math.abs(a - b) <= 1e-9 * Math.max(1, Math.abs(b))
  return JSON.stringify(a) === JSON.stringify(b)
}

/** Default values of the request fields declared in the schema. */
export function requestDefaults(schema: Schema): Values {
  const out: Values = {}
  for (const group of schema.groups)
    for (const item of group.items)
      if (item.request) out[item.key] = item.default
  return out
}

export const fmt = (v: number, digits = 1) =>
  Number(v).toLocaleString("en-US", {
    maximumFractionDigits: digits,
    minimumFractionDigits: 0,
  })
