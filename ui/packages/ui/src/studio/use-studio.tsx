import * as React from "react"

import {
  type Item,
  type Json,
  type ModelInfo,
  type Preset,
  type Schema,
  type Values,
  generate as callGenerate,
  getDefaults,
  getMesh,
  getSchema,
  getState,
  requestDefaults,
  uploadModel,
  writeValue,
} from "./api"
import type { Viewer } from "./viewer"

export type Status = { text: string; busy?: boolean; bad?: boolean; at: number }

type Options = {
  viewer: Viewer | null
  /** One line about a new result; `previous` is the result before it, if any. */
  summarize?: (out: Json, seconds: number, previous: Json | null) => string
}

/**
 * The state of a studio session: schema, parameters, the loaded model, the
 * latest result and the status line. Generation runs one at a time; a change
 * made meanwhile queues one more run.
 */
export function useStudio({ viewer, summarize }: Options) {
  const [schema, setSchema] = React.useState<Schema | null>(null)
  const [defaults, setDefaults] = React.useState<{
    params: Values
    request: Values
  } | null>(null)
  const [params, setParams] = React.useState<Values>({})
  const [request, setRequest] = React.useState<Values>({})
  const [model, setModel] = React.useState<ModelInfo | { name: string } | null>(
    null
  )
  const [result, setResult] = React.useState<Json | null>(null)
  const [shown, setShown] = React.useState<"model" | "result" | null>(null)
  const [status, setStatusState] = React.useState<Status>({ text: "", at: 0 })
  const [auto, setAuto] = React.useState(true)
  const [error, setError] = React.useState<string | null>(null)

  // Values read by callbacks that outlive a render. Edits write them at once,
  // so a run started right after an edit sends the new values.
  const live = React.useRef({
    params,
    request,
    auto,
    viewer,
    hasModel: false,
    hasResult: false,
  })
  React.useEffect(() => {
    live.current.viewer = viewer
  }, [viewer])
  const run = React.useRef({
    running: false,
    queued: false,
    timer: 0,
    previous: null as Json | null,
  })
  // The latest `generate`, for the queued re-run it starts itself.
  const again = React.useRef<() => void>(() => {})
  // A closed part looks exactly like the original from outside, so the first
  // result of each model is shown sectioned; after that the choice is the user's.
  const reveal = React.useRef({ inside: true, auto: false })

  const setStatus = React.useCallback(
    (text: string, busy = false, bad = false) => {
      setStatusState({ text, busy, bad, at: performance.now() })
    },
    []
  )

  const show = React.useCallback(
    async (which: "model" | "result", keepCamera: boolean) => {
      setShown(which)
      const v = live.current.viewer
      if (!v) return
      v.setMeshFromBinary(await getMesh(which), keepCamera)
    },
    []
  )

  const generate = React.useCallback(async () => {
    if (!live.current.hasModel) return
    if (run.current.running) {
      run.current.queued = true
      return
    }
    run.current.running = true
    run.current.queued = false
    const started = performance.now()
    const tick = window.setInterval(
      () =>
        setStatus(
          `Generating… ${((performance.now() - started) / 1000).toFixed(1)} s`,
          true
        ),
      100
    )
    setStatus("Generating…", true)
    try {
      const out = await callGenerate(live.current.params, live.current.request)
      live.current.hasResult = true
      setResult(out)
      await show("result", true)
      const v = live.current.viewer
      if (reveal.current.inside && v) {
        reveal.current.inside = false
        if (v.state.axis < 0 && !(live.current.params.open?.length > 0)) {
          v.setAxis(1)
          reveal.current.auto = true
        }
      }
      window.clearInterval(tick)
      const seconds = out.seconds ?? (performance.now() - started) / 1000
      setStatus(
        summarize
          ? summarize(out, seconds, run.current.previous)
          : `Done in ${seconds.toFixed(2)} s`
      )
      run.current.previous = out
    } catch (e) {
      window.clearInterval(tick)
      setStatus((e as Error).message, false, true)
    }
    run.current.running = false
    if (run.current.queued) again.current()
  }, [setStatus, show, summarize])
  React.useEffect(() => {
    again.current = generate
  }, [generate])

  const edited = React.useCallback(() => {
    if (!live.current.auto || !live.current.hasModel) {
      if (live.current.hasResult)
        setStatus("Parameters changed: press Generate to update.")
      return
    }
    window.clearTimeout(run.current.timer)
    run.current.timer = window.setTimeout(generate, 350)
  }, [generate, setStatus])

  const setItem = React.useCallback(
    (item: Item, value: Json) => {
      const [p, r] = writeValue(
        item,
        live.current.params,
        live.current.request,
        value
      )
      live.current.params = p
      live.current.request = r
      setParams(p)
      setRequest(r)
      // With a face left open the lattice is in plain view: drop the section
      // switched on automatically, unless it has been moved since.
      const v = live.current.viewer
      if (
        item.kind === "faces" &&
        reveal.current.auto &&
        v &&
        (value as string[]).length &&
        v.state.axis === 1 &&
        v.state.cut === 0.5
      )
        v.setAxis(-1)
      if (item.kind === "faces") reveal.current.auto = false
      edited()
    },
    [edited]
  )

  /** Applies a preset's values on top of the current parameters. */
  const applyPreset = React.useCallback(
    (preset: Preset) => {
      const p = { ...live.current.params, ...structuredClone(preset.params) }
      live.current.params = p
      setParams(p)
      edited()
    },
    [edited]
  )

  const resetAll = React.useCallback(() => {
    if (!defaults) return
    live.current.params = structuredClone(defaults.params)
    live.current.request = structuredClone(defaults.request)
    setParams(live.current.params)
    setRequest(live.current.request)
    edited()
  }, [defaults, edited])

  const upload = React.useCallback(
    async (file: File) => {
      setStatus(`Reading ${file.name}…`, true)
      try {
        const info = await uploadModel(file)
        setModel(info)
        live.current.hasModel = true
        live.current.hasResult = false
        setResult(null)
        reveal.current.inside = true
        run.current.previous = null
        await show("model", false)
        setStatus("Model loaded.")
        if (live.current.auto) generate()
      } catch (e) {
        setStatus((e as Error).message, false, true)
      }
    },
    [generate, setStatus, show]
  )

  // Start-up: the form, then whatever the server already holds (a reload).
  React.useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        const [s, d] = await Promise.all([getSchema(), getDefaults()])
        if (cancelled) return
        const r = requestDefaults(s)
        setSchema(s)
        setDefaults({ params: d, request: r })
        setParams(structuredClone(d))
        setRequest(structuredClone(r))
        live.current.params = structuredClone(d)
        live.current.request = structuredClone(r)
      } catch (e) {
        setError((e as Error).message)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  // Once both the form and the viewer exist, pick up a model already loaded.
  const resumed = React.useRef(false)
  React.useEffect(() => {
    if (!schema || resumed.current) return
    resumed.current = true
    getState()
      .then(async (state) => {
        if (!state.model) return
        setModel(state.model)
        live.current.hasModel = true
        await show("model", false)
        setStatus("Model loaded.")
        if (live.current.auto) generate()
      })
      .catch(() => {})
  }, [schema, viewer, show, setStatus, generate])

  return {
    schema,
    defaults,
    params,
    request,
    model,
    result,
    shown,
    status,
    auto,
    error,
    setAuto: (on: boolean) => {
      // Read by `edited` when the next change comes.
      live.current.auto = on
      setAuto(on)
    },
    setItem,
    applyPreset,
    resetAll,
    upload,
    generate,
    show,
    /** Shows a message in the status line and toast. */
    setStatus,
  }
}

export type Studio = ReturnType<typeof useStudio>
