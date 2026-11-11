import * as React from "react"
import { cn } from "cn"
import { ChevronRight } from "lucide-react"

import { Button } from "@workspace/ui/components/button"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@workspace/ui/components/collapsible"
import { Slider } from "@workspace/ui/components/slider"
import {
  ToggleGroup,
  ToggleGroupItem,
} from "@workspace/ui/components/toggle-group"
import { InfoTip } from "./info-tip"
import { Hint, PanelHeading } from "./panel"
import type { ViewName, Viewer } from "./viewer"

/**
 * A row of mutually exclusive options. One track holds borderless options;
 * the chosen one is a tinted pill with its own inset ring, so no border is
 * ever shared between neighbours (shared borders leave the chosen option
 * with a grey side).
 */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  disabled,
  className,
}: {
  value: T
  options: [T, React.ReactNode][]
  onChange: (value: T) => void
  disabled?: boolean | ((value: T) => boolean)
  className?: string
}) {
  return (
    <ToggleGroup
      type="single"
      size="sm"
      spacing={0.5}
      value={value}
      // Radix sends "" when the pressed item is clicked again: keep the value.
      onValueChange={(v) => v && onChange(v as T)}
      className={cn(
        "h-8 w-full rounded-lg border border-line bg-field p-[3px]",
        className
      )}
    >
      {options.map(([v, label]) => (
        <ToggleGroupItem
          key={v}
          value={v}
          className="h-full min-w-0 flex-1 rounded-md px-2 transition-colors duration-150 hover:bg-raised data-[state=on]:shadow-[inset_0_0_0_1px_rgb(61_190_139/0.5)] motion-reduce:transition-none"
          disabled={typeof disabled === "function" ? disabled(v) : disabled}
        >
          {label}
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  )
}

/**
 * Section, shading, colour and camera controls for a viewer, with keyboard
 * shortcuts. `version` must change when the viewer's state does.
 */
export function ViewControls({
  viewer,
  layerHeight,
}: {
  viewer: Viewer | null
  version: number
  layerHeight?: number
}) {
  useViewerKeys(viewer, layerHeight)
  if (!viewer) return null
  const state = viewer.state
  const mesh = viewer.bounds()
  let cutLabel = "No section"
  if (state.axis >= 0 && mesh) {
    const pos = viewer.cutPosition()
    const layer =
      state.axis === 2 && layerHeight
        ? ` · layer ${Math.max(1, Math.ceil((pos - mesh.lo[2]) / layerHeight))} at ${layerHeight} mm`
        : ""
    cutLabel = `${"XYZ"[state.axis]} = ${pos.toFixed(2)} mm${layer}`
  }
  return (
    <div>
      <PanelHeading>Section</PanelHeading>
      <div className="flex gap-2">
        <Segmented
          value={String(state.axis)}
          options={[
            ["-1", "Off"],
            ["0", "X"],
            ["1", "Y"],
            ["2", "Z"],
          ]}
          onChange={(v) => viewer.setAxis(+v)}
        />
        <Button
          variant="outline"
          size="default"
          title="Keep the other half (F)"
          disabled={state.axis < 0 || !mesh}
          onClick={() => viewer.update({ flip: !state.flip })}
        >
          Flip
        </Button>
      </div>
      <Slider
        className="mt-3 mb-1.5"
        min={0}
        max={1000}
        step={1}
        disabled={state.axis < 0 || !mesh}
        value={[Math.round(state.cut * 1000)]}
        onValueChange={([v]) => viewer.update({ cut: v / 1000 })}
      />
      <Hint className="text-muted-foreground">{cutLabel}</Hint>

      <PanelHeading>Shading</PanelHeading>
      <Segmented
        value={state.smooth ? "smooth" : "flat"}
        options={[
          ["flat", "Flat"],
          ["smooth", "Smooth"],
        ]}
        onChange={(v) => viewer.update({ smooth: v === "smooth" })}
      />

      <PanelHeading className="flex items-center gap-1.5">
        Colour
        <InfoTip label="About the colours">
          Overhang paints down-facing surfaces by their angle from vertical,
          with Z as the print direction. Section the part to see the inside.
        </InfoTip>
      </PanelHeading>
      <Segmented
        value={String(state.mode)}
        options={[
          ["0", "Material"],
          ["1", "Overhang"],
        ]}
        onChange={(v) => viewer.update({ mode: +v })}
      />
      {state.mode === 1 && (
        <>
          <div className="mt-1.5 flex overflow-hidden rounded-sm text-center text-[11px] text-canvas">
            <span className="flex-1 bg-oh-safe py-0.5">&lt;30°</span>
            <span className="flex-1 bg-oh-mild py-0.5">30–45°</span>
            <span className="flex-1 bg-oh-steep py-0.5">45–60°</span>
            <span className="flex-1 bg-oh-bad py-0.5">&gt;60°</span>
          </div>
        </>
      )}

      <PanelHeading>View</PanelHeading>
      <div className="grid grid-cols-4 gap-1">
        {(["iso", "top", "front", "side"] as ViewName[]).map((v) => (
          <Button
            key={v}
            variant="outline"
            size="default"
            onClick={() => viewer.setView(v)}
            className="capitalize"
          >
            {v === "iso" ? "Iso" : v}
          </Button>
        ))}
      </div>

      <Collapsible className="mt-3">
        <CollapsibleTrigger className="group/trigger flex items-center gap-1.5 font-medium text-muted-foreground outline-none hover:text-foreground">
          <ChevronRight className="size-3.5 transition-transform duration-200 ease-out-strong group-data-[state=open]/trigger:rotate-90 motion-reduce:transition-none" />
          Mouse and keys
        </CollapsibleTrigger>
        <CollapsibleContent>
          <p className="mt-1.5 leading-relaxed text-muted-foreground">
            Drag to orbit · right-drag or <kbd>Shift</kbd>+drag to pan · wheel
            to zoom · double-click to re-centre · <kbd>X</kbd> <kbd>Y</kbd>{" "}
            <kbd>Z</kbd> section · <kbd>←</kbd> <kbd>→</kbd> move it ·{" "}
            <kbd>F</kbd> flip · <kbd>O</kbd> overhang · <kbd>S</kbd> shading ·{" "}
            <kbd>R</kbd> reset view
          </p>
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}

/** Keyboard shortcuts of the view, ignored while typing in a field. */
export function useViewerKeys(viewer: Viewer | null, layerHeight?: number) {
  React.useEffect(() => {
    if (!viewer) return
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement
      if (
        e.ctrlKey ||
        e.metaKey ||
        e.altKey ||
        /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName) ||
        target.isContentEditable
      )
        return
      const state = viewer.state
      const k = e.key.toLowerCase()
      if (k === "x" || k === "y" || k === "z") {
        const a = "xyz".indexOf(k)
        viewer.setAxis(state.axis === a ? -1 : a)
        return
      }
      if (k === "f") viewer.update({ flip: !state.flip })
      else if (k === "o") viewer.update({ mode: 1 - state.mode })
      else if (k === "s") viewer.update({ smooth: !state.smooth })
      else if (k === "r") {
        viewer.setView("iso")
        viewer.setAxis(-1)
        viewer.update({ cut: 0.5, flip: false })
      } else if (k === "arrowleft" || k === "arrowright") {
        const mesh = viewer.bounds()
        if (state.axis < 0 || !mesh) return
        e.preventDefault()
        const step =
          (e.shiftKey ? 5 : 1) *
          (state.axis === 2 && layerHeight ? layerHeight / mesh.size[2] : 0.002)
        viewer.update({
          cut: Math.max(
            0,
            Math.min(1, state.cut + (k === "arrowright" ? step : -step))
          ),
        })
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [viewer, layerHeight])
}
