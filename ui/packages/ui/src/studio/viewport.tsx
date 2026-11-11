import * as React from "react"
import { cn } from "cn"

import { Spinner } from "./panel"
import type { Status } from "./use-studio"

/**
 * The 3D view: the canvas, a status toast that stays in view while work is in
 * progress and fades a few seconds after a result, and an overlay (shown
 * when `children` are given) for the empty state.
 *
 * Motion: the canvas fades in when it first has something to show (once per
 * model, so the model does not pop in); the toast drops in by a few pixels
 * and leaves with a plain fade, quicker than it came.
 */
export function Viewport({
  canvasRef,
  status,
  showing = true,
  placing = false,
  children,
  className,
}: {
  canvasRef: React.RefObject<HTMLCanvasElement | null>
  status: Status
  /** Whether the canvas has a mesh to show. */
  showing?: boolean
  /** Waiting for a click on the part: crosshair cursor. */
  placing?: boolean
  children?: React.ReactNode
  className?: string
}) {
  // A status shows until it is done and a few seconds have passed; each new
  // status (a new `at`) shows again.
  const [faded, setFaded] = React.useState(0)
  React.useEffect(() => {
    if (!status.text || status.busy) return
    const timer = window.setTimeout(
      () => setFaded(status.at),
      status.bad ? 9000 : 3500
    )
    return () => window.clearTimeout(timer)
  }, [status])
  const visible = !!status.text && faded !== status.at

  return (
    <main className={cn("relative min-w-0 overflow-hidden", className)}>
      <canvas
        ref={canvasRef}
        className={cn(
          "block size-full cursor-grab touch-none transition-[filter,opacity] duration-300 ease-out-strong motion-reduce:transition-none [&.dragging]:cursor-grabbing",
          status.busy && "brightness-[0.82]",
          showing ? "opacity-100" : "opacity-0",
          placing && "cursor-crosshair"
        )}
      />
      <div
        role="status"
        aria-live="polite"
        className={cn(
          "pointer-events-none absolute top-3.5 left-1/2 flex max-w-[calc(100%-28px)] -translate-x-1/2 items-center gap-2 rounded-full border bg-surface/95 px-4 py-[7px] whitespace-nowrap tabular-nums shadow-[0_4px_18px_rgb(0_0_0/0.45)] transition-[opacity,translate,border-color] motion-reduce:transition-none",
          visible
            ? "translate-y-0 opacity-100 duration-200 ease-out-strong"
            : "-translate-y-1 opacity-0 duration-150 ease-out",
          status.bad
            ? "border-destructive text-destructive"
            : status.busy
              ? "border-jade"
              : "border-line"
        )}
      >
        {status.busy && <Spinner />}
        <span className="truncate">{status.text}</span>
      </div>
      {children && (
        <div className="absolute inset-0 flex items-center justify-center p-6 text-center">
          {children}
        </div>
      )}
    </main>
  )
}
