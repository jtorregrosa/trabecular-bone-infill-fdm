import * as React from "react"

import { Download, RotateCcw } from "lucide-react"

import { Button, buttonVariants } from "@workspace/ui/components/button"
import { Checkbox } from "@workspace/ui/components/checkbox"
import { cn } from "@workspace/ui/lib/utils"
import { downloadUrl, fmt } from "@workspace/ui/studio/api"
import { DropZone, useFileDrop } from "@workspace/ui/studio/file-drop"
import {
  Panel,
  PanelBody,
  PanelFooter,
  PanelHeading,
  Spinner,
} from "@workspace/ui/studio/panel"
import { ParamForm } from "@workspace/ui/studio/param-form"
import { PresetPicker } from "@workspace/ui/studio/preset-picker"
import { useStudio } from "@workspace/ui/studio/use-studio"
import { Segmented, ViewControls } from "@workspace/ui/studio/view-controls"
import { ViewerContext, useViewer } from "@workspace/ui/studio/viewer"
import { Viewport } from "@workspace/ui/studio/viewport"

import { ContestBanner } from "./contest-banner"
import { Logo } from "./logo"
import { AllFigures, Headline, summarize } from "./report"

export function App() {
  const { canvasRef, viewer, failed, version, handle } = useViewer()
  const studio = useStudio({ viewer, summarize })
  const { dragging, pick } = useFileDrop(studio.upload)
  const layerHeight: number = studio.request.layer_height ?? 0.2
  const product = studio.schema?.product ?? { name: "bone-infill", tagline: "" }
  const stem = (studio.model?.name ?? "model").replace(/\.[^.]+$/, "")
  const info = studio.model && "size" in studio.model ? studio.model : null

  // The sample is a proximal femur, served next to the page and fetched only
  // when asked for (examples/sample_model.rs makes it).
  const loadSample = async () => {
    studio.setStatus("Downloading the sample…", true)
    try {
      const response = await fetch("sample.stl")
      if (!response.ok)
        throw new Error(`the sample is not available (${response.status})`)
      const file = new File([await response.blob()], "femur-proximal.stl", {
        type: "model/stl",
      })
      await studio.upload(file)
      // Seen from the side, cut through the middle along X: the cut face
      // looks straight at the camera, with the lattice in it.
      if (viewer) {
        viewer.setView("side")
        viewer.setAxis(0)
        viewer.update({ cut: 0.5 })
      }
    } catch (e) {
      studio.setStatus((e as Error).message, false, true)
    }
  }

  // The loading screen (index.html) fades out once the form can be shown.
  const loaded = !!studio.schema || !!studio.error
  React.useEffect(() => {
    const splash = document.getElementById("splash")
    if (!loaded || !splash) return
    splash.classList.add("done")
    const timer = window.setTimeout(() => splash.remove(), 400)
    return () => window.clearTimeout(timer)
  }, [loaded])

  // Reinforced spheres and the load direction are drawn through the part.
  const loads = studio.params.loads
  const direction = studio.params.stretch_dir
  React.useEffect(() => {
    viewer?.setOverlays({ loads: loads ?? [], direction: direction ?? null })
  }, [viewer, loads, direction])

  return (
    <ViewerContext.Provider value={handle}>
      <div className="fixed inset-0 flex flex-col">
        <ContestBanner />
        <div className="grid min-h-0 flex-1 grid-cols-[300px_minmax(0,1fr)_250px] xl:grid-cols-[348px_minmax(0,1fr)_300px]">
          {/* Left: what goes in. */}
          <Panel className="border-r border-line">
            <PanelBody>
              <h1 className="flex items-center gap-2.5 text-base font-semibold">
                <Logo className="h-7" />
                {product.name}
              </h1>
              <p className="text-muted-foreground">{product.tagline}</p>

              <PanelHeading>Model</PanelHeading>
              <DropZone dragging={dragging} onClick={pick}>
                {studio.model
                  ? studio.model.name
                  : "Drop an STL or 3MF here, or click to choose"}
              </DropZone>
              {info && (
                <div className="mt-1.5 text-muted-foreground">
                  {fmt(info.size[0])} × {fmt(info.size[1])} ×{" "}
                  {fmt(info.size[2])} mm · {fmt(info.volume / 1000)} cm³ ·{" "}
                  {info.triangles.toLocaleString("en-US")} triangles
                  {info.notes.map((note) => (
                    <div key={note} className="mt-1 text-destructive">
                      Note: {note}
                    </div>
                  ))}
                </div>
              )}

              <div className="mt-[18px] mb-2 flex items-center justify-between">
                <PanelHeading className="my-0">Parameters</PanelHeading>
                <Button
                  variant="ghost"
                  size="xs"
                  className="-mr-2 text-muted-foreground"
                  title="Every parameter back to its default"
                  onClick={studio.resetAll}
                >
                  <RotateCcw aria-hidden="true" />
                  Reset all
                </Button>
              </div>
              {studio.schema && studio.defaults ? (
                <>
                  <PresetPicker
                    presets={studio.schema.presets ?? []}
                    params={studio.params}
                    onPick={studio.applyPreset}
                  />
                  <ParamForm
                    groups={studio.schema.groups}
                    params={studio.params}
                    request={studio.request}
                    defaults={studio.defaults}
                    onChange={studio.setItem}
                    result={studio.result}
                  />
                </>
              ) : (
                <p
                  className={
                    studio.error ? "text-destructive" : "text-muted-foreground"
                  }
                >
                  {studio.error ?? "Loading…"}
                </p>
              )}
            </PanelBody>
            {/* The action that turns the configuration into a result. */}
            <PanelFooter className="flex flex-col gap-2 pt-3">
              <Button
                size="lg"
                className="w-full font-semibold"
                disabled={!studio.model}
                onClick={studio.generate}
              >
                {studio.status.busy ? (
                  <Spinner className="border-on-jade/30 border-t-on-jade" />
                ) : null}
                Generate
              </Button>
              <label
                className="flex items-center gap-2 text-muted-foreground"
                title="Generate again a moment after each change"
              >
                <Checkbox
                  checked={studio.auto}
                  onCheckedChange={(c) => studio.setAuto(c === true)}
                />
                Update automatically
              </label>
              {studio.status.bad && (
                <p className="text-destructive">{studio.status.text}</p>
              )}
            </PanelFooter>
          </Panel>

          <Viewport
            canvasRef={canvasRef}
            status={studio.status}
            showing={!!studio.shown}
            placing={handle.placing}
          >
            {failed ? (
              <p className="text-subtle">
                This app needs WebGL 2 to show the model. Generating and
                downloading still work.
              </p>
            ) : !studio.shown ? (
              <EmptyState
                product={product}
                dragging={dragging}
                onPick={pick}
                onSample={loadSample}
              />
            ) : null}
          </Viewport>

          {/* Right: the output. How it is shown, and at the foot what came out
          and the files to take away. */}
          <Panel className="border-l border-line">
            <PanelBody>
              <PanelHeading className="mt-0">Show</PanelHeading>
              <Segmented
                value={studio.shown ?? "result"}
                options={[
                  ["result", "Result"],
                  ["model", "Original"],
                ]}
                disabled={(v) =>
                  v === "result" ? !studio.result : !studio.model
                }
                onChange={(v) => studio.show(v, true)}
              />
              <ViewControls
                viewer={viewer}
                version={version}
                layerHeight={layerHeight}
              />
            </PanelBody>
            <PanelFooter className="pt-3">
              <section aria-label="Result" className="flex flex-col gap-2">
                <PanelHeading className="my-0">Result</PanelHeading>
                <Headline out={studio.result} />
                <AllFigures
                  out={studio.result}
                  viewer={viewer}
                  layerHeight={layerHeight}
                />
                <div className="grid grid-cols-2 gap-2">
                  <DownloadLink
                    ready={!!studio.result}
                    format="3mf"
                    file={`${stem}_bone.3mf`}
                    title="For Bambu Studio, OrcaSlicer and PrusaSlicer"
                  />
                  <DownloadLink
                    ready={!!studio.result}
                    format="stl"
                    file={`${stem}_bone.stl`}
                  />
                </div>
              </section>
            </PanelFooter>
          </Panel>
        </div>
      </div>
    </ViewerContext.Provider>
  )
}

/** First visit: the canvas itself invites a file, or the sample. */
function EmptyState({
  product,
  dragging,
  onPick,
  onSample,
}: {
  product: { name: string; tagline: string }
  dragging: boolean
  onPick: () => void
  onSample: () => void
}) {
  return (
    // Seen once per visit, so it may arrive with a little more presence than
    // the rest of the interface.
    <div
      className={cn(
        "max-w-[380px] rounded-2xl border border-dashed border-line-strong px-7 pt-7 pb-6 transition-colors duration-150",
        "animate-in duration-300 ease-out-strong fade-in-0 slide-in-from-bottom-2 motion-reduce:animate-none",
        dragging && "border-jade bg-jade-tint"
      )}
    >
      {/* The lockup of the brand pack: the bone with the name beside it. */}
      <div className="mx-auto mb-6 flex w-fit items-center gap-4 text-left">
        <Logo className="h-32" />
        <div>
          <div className="text-[30px] leading-none font-semibold tracking-tight">
            {product.name}
          </div>
          <div className="mt-2 max-w-[165px] text-muted-foreground">
            {product.tagline}
          </div>
        </div>
      </div>
      <h2 className="mb-1 text-[17px] font-semibold">
        Fill a part with printable bone
      </h2>
      <p className="text-muted-foreground">Drop an STL or 3MF here, or</p>
      <div className="my-4 flex justify-center gap-2">
        <Button onClick={onPick}>Choose a file</Button>
        <Button variant="outline" onClick={onSample}>
          Try a sample
        </Button>
      </div>
      <p className="text-xs text-subtle">
        Z is the print direction: orient the part before loading it.
        <br />
        Your file stays on this computer.
      </p>
    </div>
  )
}

/** A plain link, so that the WebAssembly build can intercept it. */
function DownloadLink({
  ready,
  format,
  file,
  title,
}: {
  ready: boolean
  format: string
  file: string
  title?: string
}) {
  return (
    <a
      href={downloadUrl(format)}
      download={file}
      aria-label={`Download ${format.toUpperCase()}`}
      title={title}
      aria-disabled={!ready}
      className={cn(
        buttonVariants({ variant: "outline" }),
        "flex-1",
        !ready && "pointer-events-none opacity-45"
      )}
    >
      <Download aria-hidden="true" />
      {format.toUpperCase()}
    </a>
  )
}
