import * as React from "react"

import coreSource from "./viewer-core.js?raw"

export type ViewName = "iso" | "top" | "front" | "side"

/** View state; read it freely, change it with `update`. */
export type ViewState = {
  /** Section axis 0, 1, 2, or -1 for none. */
  axis: number
  /** Section position along the axis, 0..1 of the model's extent. */
  cut: number
  /** Keep the other half. */
  flip: boolean
  /** 0 material, 1 overhang colouring. */
  mode: number
  smooth: boolean
}

export type MeshBounds = {
  count: number
  lo: number[]
  size: number[]
  centre: number[]
  radius: number
}

export type Viewer = {
  state: ViewState
  setMeshFromBinary(buffer: ArrayBuffer, keepCamera?: boolean): unknown
  setView(name: ViewName): void
  setAxis(axis: number): void
  cutAt(axis: number, position: number): void
  update(
    patch: Partial<Pick<ViewState, "cut" | "flip" | "mode" | "smooth">>
  ): void
  changed(): void
  redraw(): void
  cutPosition(): number
  bounds(): MeshBounds | null
  /** Point of the part under a page position; on a cut face, the point of the cut. */
  pickClient(clientX: number, clientY: number): Point | null
  /** Markers drawn through the part. */
  setOverlays(patch: Partial<Overlays>): void
  dispose(): void
}

export type Point = [number, number, number]
export type Sphere = { center: Point; radius: number }
export type Overlays = {
  loads: Sphere[]
  /** Index of the sphere being edited, drawn brighter; -1 for none. */
  selected: number
  /** A direction drawn as a double-headed arrow through the part. */
  direction: Point | null
}

/**
 * What a form needs from the viewer: the viewer itself, and a way to ask the
 * user for a point on the part. `requestPoint` puts the view in a placing
 * mode (crosshair cursor); the next click on the part, without dragging,
 * answers it. Escape or `cancelPoint` leaves the mode.
 */
export type ViewerHandle = {
  viewer: Viewer | null
  requestPoint(onPoint: (point: Point) => void): void
  cancelPoint(): void
  placing: boolean
}

export const ViewerContext = React.createContext<ViewerHandle>({
  viewer: null,
  requestPoint: () => {},
  cancelPoint: () => {},
  placing: false,
})

export const useViewerHandle = () => React.useContext(ViewerContext)

type CreateViewer = (
  canvas: HTMLCanvasElement,
  opts?: { onChange?: () => void; leftInset?: () => number }
) => Viewer | null

// The renderer is a classic script shared with the standalone viewer page of
// the Rust crate; evaluate it once and keep its factory.
let factory: CreateViewer | null = null
function createViewer(
  canvas: HTMLCanvasElement,
  onChange: () => void
): Viewer | null {
  factory ??= new Function(
    `${coreSource}\nreturn createViewer;`
  )() as CreateViewer
  return factory(canvas, { onChange })
}

/**
 * Creates a viewer on the returned canvas ref. `version` changes whenever the
 * view state does, so components that show it re-render.
 */
export function useViewer() {
  const canvasRef = React.useRef<HTMLCanvasElement>(null)
  const [viewer, setViewer] = React.useState<Viewer | null>(null)
  const [failed, setFailed] = React.useState(false)
  const [version, bump] = React.useReducer((n: number) => n + 1, 0)

  React.useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    let created: Viewer | null = null
    try {
      created = createViewer(canvas, bump)
    } catch {
      created = null
    }
    setViewer(created)
    setFailed(!created)
    return () => created?.dispose()
  }, [])

  // Placing mode: a click that does not move (an orbit drag does) answers
  // the pending request with the point under it.
  const pending = React.useRef<((point: Point) => void) | null>(null)
  const [placing, setPlacing] = React.useState(false)
  const cancelPoint = React.useCallback(() => {
    pending.current = null
    setPlacing(false)
  }, [])
  const requestPoint = React.useCallback((onPoint: (point: Point) => void) => {
    pending.current = onPoint
    setPlacing(true)
  }, [])
  React.useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas || !viewer || !placing) return
    let down: { x: number; y: number } | null = null
    const onDown = (e: PointerEvent) => {
      down = e.button === 0 ? { x: e.clientX, y: e.clientY } : null
    }
    const onUp = (e: PointerEvent) => {
      if (!down || Math.hypot(e.clientX - down.x, e.clientY - down.y) > 4)
        return
      const point = viewer.pickClient(e.clientX, e.clientY)
      if (!point || !pending.current) return
      const answer = pending.current
      cancelPoint()
      answer(point)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") cancelPoint()
    }
    canvas.addEventListener("pointerdown", onDown)
    canvas.addEventListener("pointerup", onUp)
    window.addEventListener("keydown", onKey)
    return () => {
      canvas.removeEventListener("pointerdown", onDown)
      canvas.removeEventListener("pointerup", onUp)
      window.removeEventListener("keydown", onKey)
    }
  }, [viewer, placing, cancelPoint])

  const handle = React.useMemo<ViewerHandle>(
    () => ({ viewer, requestPoint, cancelPoint, placing }),
    [viewer, requestPoint, cancelPoint, placing]
  )
  return { canvasRef, viewer, failed, version, handle }
}
