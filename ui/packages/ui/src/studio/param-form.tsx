import * as React from "react"
import { cn } from "cn"
import {
  ChevronRight,
  Plus,
  RefreshCw,
  RotateCcw,
  Trash2,
  TriangleAlert,
} from "lucide-react"

import { Button } from "@workspace/ui/components/button"
import { Checkbox } from "@workspace/ui/components/checkbox"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@workspace/ui/components/collapsible"
import { Input } from "@workspace/ui/components/input"
import { Slider } from "@workspace/ui/components/slider"
import { Toggle } from "@workspace/ui/components/toggle"
import {
  type Group,
  type Item,
  type Json,
  type Values,
  readValue,
  sameValue,
} from "./api"
import { InfoCard, InfoTip } from "./info-tip"
import { Segmented } from "./view-controls"
import { type Point, type Sphere, useViewerHandle } from "./viewer"

type FormProps = {
  groups: Group[]
  params: Values
  request: Values
  defaults: { params: Values; request: Values }
  onChange: (item: Item, value: Json) => void
  /** The latest generate reply, for fields that report on their effect. */
  result?: Json | null
}

/**
 * The parameter form, built from the schema. A parameter's explanation sits
 * behind an info icon next to its name. A parameter that differs from its
 * default carries a jade dot and a reset button; a group shows how many of
 * its parameters are changed.
 */
export function ParamForm({
  groups,
  params,
  request,
  defaults,
  onChange,
  result,
}: FormProps) {
  const isChanged = (item: Item) =>
    !sameValue(
      readValue(item, params, request),
      readValue(item, defaults.params, defaults.request)
    )
  return (
    <div className="flex flex-col">
      {groups.map((group) => {
        const changed = group.items.filter(isChanged).length
        return (
          <Collapsible
            key={group.title}
            defaultOpen={!!group.open}
            className="border-t border-line last:border-b"
          >
            <CollapsibleTrigger className="group/trigger flex w-full items-center gap-1.5 py-[7px] text-left font-semibold outline-none focus-visible:text-jade-hover">
              <ChevronRight className="size-3.5 text-subtle transition-transform duration-200 ease-out-strong group-data-[state=open]/trigger:rotate-90 motion-reduce:transition-none" />
              {group.title}
              {changed > 0 && (
                <span className="ml-1.5 text-xs font-normal text-jade-hover">
                  {changed} changed
                </span>
              )}
            </CollapsibleTrigger>
            {/* Opens with a short settle from above; closes at once. */}
            <CollapsibleContent className="pb-2 pl-3 duration-200 ease-out-strong data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:slide-in-from-top-1 motion-reduce:animate-none">
              {group.items.map((item) => (
                <Field
                  key={item.key}
                  item={item}
                  value={readValue(item, params, request)}
                  changed={isChanged(item)}
                  context={{ params, result: result ?? null }}
                  onChange={(v) => onChange(item, v)}
                  onReset={() =>
                    onChange(
                      item,
                      structuredClone(
                        readValue(item, defaults.params, defaults.request)
                      )
                    )
                  }
                />
              ))}
            </CollapsibleContent>
          </Collapsible>
        )
      })}
    </div>
  )
}

type FieldProps = {
  item: Item
  value: Json
  changed: boolean
  onChange: (value: Json) => void
  onReset: () => void
  /** The whole parameter set and the latest result, for fields that need them. */
  context?: { params: Values; result: Json | null }
}

function Label({
  item,
  changed,
  onReset,
  htmlFor,
  className,
}: Omit<FieldProps, "value" | "onChange"> & {
  htmlFor?: string
  className?: string
}) {
  return (
    <label
      htmlFor={htmlFor}
      className={cn("flex min-w-0 items-center gap-1.5", className)}
    >
      {/* Changed marker: grows in, so a change made elsewhere (a preset,
          Reset all) is noticed without drawing attention for long. */}
      <span
        aria-hidden="true"
        className={cn(
          "-ml-3 size-1.5 shrink-0 rounded-full bg-jade transition-[opacity,scale] duration-150 ease-out-strong motion-reduce:transition-none",
          changed ? "scale-100 opacity-100" : "scale-50 opacity-0"
        )}
      />
      {/* Long names wrap rather than being cut. */}
      <span className="min-w-0 leading-tight text-pretty">{item.label}</span>
      {(item.about || item.effects || item.tip) && (
        <InfoTip label={`About ${item.label}`}>
          <InfoCard
            title={item.label}
            about={item.about}
            effects={item.effects}
            tip={item.tip}
          />
        </InfoTip>
      )}
      <button
        type="button"
        title="Back to the default"
        aria-label={`Reset ${item.label}`}
        tabIndex={changed ? 0 : -1}
        onClick={(e) => {
          e.preventDefault()
          onReset()
        }}
        className={cn(
          "inline-flex size-4 shrink-0 items-center justify-center rounded-full text-subtle transition-[opacity,color] duration-150 outline-none hover:text-foreground focus-visible:text-jade-hover motion-reduce:transition-none",
          changed ? "opacity-100" : "pointer-events-none opacity-0"
        )}
      >
        <RotateCcw className="size-3" />
      </button>
    </label>
  )
}

function Field(props: FieldProps) {
  const { item } = props
  const id = "p_" + item.key.replace(".", "_")
  const wrap =
    "mb-[9px] grid grid-cols-[1fr_76px] items-center gap-x-2 gap-y-0.5"
  switch (item.kind) {
    case "bool":
      return (
        <div className="mb-[9px]">
          <div className="flex items-center gap-2">
            <Checkbox
              id={id}
              checked={!!props.value}
              onCheckedChange={(c) => props.onChange(c === true)}
            />
            <Label {...props} htmlFor={id} />
          </div>
        </div>
      )
    case "seed":
      return (
        <div className={wrap}>
          <div className="flex min-w-0 items-center justify-between gap-1">
            <Label {...props} htmlFor={id} />
            {/* A new random layout: the circular arrow reads as "roll again". */}
            <Button
              type="button"
              variant="outline"
              size="icon-sm"
              title="New pattern"
              aria-label="New pattern"
              onClick={() => props.onChange(Math.floor(Math.random() * 100000))}
              className="group/new"
            >
              <RefreshCw className="transition-transform duration-300 ease-out-strong group-active/new:rotate-180 motion-reduce:transition-none" />
            </Button>
          </div>
          <CommitInput
            id={id}
            type="number"
            value={String(props.value)}
            onCommit={(s) => props.onChange(Math.max(0, Math.round(+s) || 0))}
          />
        </div>
      )
    case "faces":
      return <Faces {...props} />
    case "loads":
      return <LoadsField {...props} />
    case "vec3":
      return <DirectionField {...props} id={id} />
    case "optional":
      return (
        <div className={wrap}>
          <Label {...props} htmlFor={id} />
          <CommitInput
            id={id}
            type="number"
            value={
              props.value === null || props.value === undefined
                ? ""
                : String(props.value)
            }
            placeholder={item.placeholder}
            onCommit={(s) =>
              props.onChange(s.trim() === "" ? null : Math.max(0, +s))
            }
          />
        </div>
      )
    default:
      return <NumberField {...props} id={id} />
  }
}

function NumberField({
  item,
  value,
  onChange,
  id,

  ...rest
}: FieldProps & { id: string }) {
  const scale = item.scale ?? 1
  const shown = +((value as number) * scale).toFixed(6)
  // While a slider is dragged, only the box follows; the value is committed
  // (and the part regenerated) when the slider is let go.
  const [dragging, setDragging] = React.useState<number | null>(null)
  const commit = (raw: number) => {
    if (!Number.isFinite(raw)) return
    const v = item.integer ? Math.round(raw) : raw
    onChange(v / scale)
  }
  return (
    <div className="mb-[9px] grid grid-cols-[1fr_76px] items-center gap-x-2 gap-y-1">
      <Label item={item} {...rest} htmlFor={id} />
      <CommitInput
        id={id}
        type="number"
        value={String(dragging ?? shown)}
        onCommit={(s) => commit(Number(s))}
        min={item.min}
        max={item.max}
        step={item.step}
      />
      <Slider
        className="col-span-2 py-1.5"
        min={item.min}
        max={item.max}
        step={item.step}
        value={[dragging ?? shown]}
        onValueChange={([v]) => setDragging(v)}
        onValueCommit={([v]) => {
          setDragging(null)
          commit(v)
        }}
        tabIndex={-1}
      />
    </div>
  )
}

type LoadZones = {
  inside: number
  unreinforced: number
  outside: number
  covered: number[]
}

const round1 = (v: number) => Math.round(v * 10) / 10
const percent = (v: number) => `${(100 * v).toFixed(1)} %`

/**
 * Reinforced spheres: Add puts the view in placing mode, and a click on the
 * part (or on a cut face, to reach the inside) drops a sphere there. Each
 * sphere is a row of X, Y, Z and radius; hovering or editing a row lights its
 * sphere in the view. After a run, the field reports what the spheres did
 * and flags any that reach no lattice.
 */
function LoadsField(props: FieldProps) {
  const { viewer, requestPoint, cancelPoint, placing } = useViewerHandle()
  const loads = (props.value ?? []) as Sphere[]
  const [selected, setSelected] = React.useState(-1)
  React.useEffect(() => {
    viewer?.setOverlays({ selected })
  }, [viewer, selected])
  // A request left open when the field goes away would answer nobody.
  React.useEffect(() => () => cancelPoint(), [cancelPoint])

  const cell = Number(props.context?.params.cell) || 16
  const radius = Math.max(2, Math.round(cell / 2))
  const report = props.context?.result?.report
  // Figures only when they describe these spheres, not an earlier set.
  const zones: LoadZones | null =
    report?.load_zones && sameValue(report.params.loads, loads)
      ? report.load_zones
      : null
  const update = (i: number, patch: Partial<Sphere>) =>
    props.onChange(loads.map((l, n) => (n === i ? { ...l, ...patch } : l)))
  const add = () => {
    if (placing) return cancelPoint()
    requestPoint((point) => {
      props.onChange([...loads, { center: point.map(round1) as Point, radius }])
      setSelected(loads.length)
    })
  }
  const columns = "grid grid-cols-[repeat(4,minmax(0,1fr))_24px] gap-1"

  return (
    <div className="mb-[9px] flex flex-col gap-1.5">
      <div className="flex items-center justify-between gap-2">
        <Label {...props} />
        <Button
          type="button"
          size="xs"
          variant={placing ? "default" : "outline"}
          disabled={!viewer?.bounds()}
          onClick={add}
        >
          {placing ? (
            "Cancel"
          ) : (
            <>
              <Plus aria-hidden="true" />
              Add
            </>
          )}
        </Button>
      </div>
      {placing && (
        <p className="text-xs text-jade-hover">
          Click the part, or a cut face to place it inside. Esc cancels.
        </p>
      )}
      {loads.length > 0 && (
        <div className="flex flex-col gap-1">
          <div className={cn(columns, "px-px text-[11px] text-subtle")}>
            <span>X</span>
            <span>Y</span>
            <span>Z</span>
            <span>Radius</span>
          </div>
          {loads.map((l, i) => (
            <React.Fragment key={i}>
              <div
                className={cn(
                  columns,
                  "items-center rounded-md transition-colors duration-150 motion-reduce:transition-none",
                  selected === i && "bg-jade-tint"
                )}
                onMouseEnter={() => setSelected(i)}
                onMouseLeave={() => setSelected(-1)}
                onFocusCapture={() => setSelected(i)}
                onBlurCapture={() => setSelected(-1)}
              >
                {[0, 1, 2].map((a) => (
                  <CommitInput
                    key={a}
                    type="number"
                    step={0.5}
                    aria-label={`Sphere ${i + 1} ${"XYZ"[a]} (mm)`}
                    value={String(l.center[a])}
                    onCommit={(s) => {
                      const v = Number(s)
                      if (!Number.isFinite(v)) return
                      const center = [...l.center] as Point
                      center[a] = v
                      update(i, { center })
                    }}
                    className="px-1 text-xs"
                  />
                ))}
                <CommitInput
                  type="number"
                  step={0.5}
                  min={0.5}
                  aria-label={`Sphere ${i + 1} radius (mm)`}
                  value={String(l.radius)}
                  onCommit={(s) => {
                    const v = Number(s)
                    if (Number.isFinite(v) && v > 0) update(i, { radius: v })
                  }}
                  className="px-1 text-xs"
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  aria-label={`Remove sphere ${i + 1}`}
                  title="Remove"
                  onClick={() => {
                    setSelected(-1)
                    props.onChange(loads.filter((_, n) => n !== i))
                  }}
                >
                  <Trash2 aria-hidden="true" />
                </Button>
              </div>
              {zones && zones.covered[i] === 0 && (
                <p className="flex items-start gap-1 text-xs text-destructive">
                  <TriangleAlert
                    aria-hidden="true"
                    className="mt-px size-3 shrink-0"
                  />
                  Sphere {i + 1} reaches no lattice: it is outside the part or
                  only in its skin.
                </p>
              )}
            </React.Fragment>
          ))}
        </div>
      )}
      {zones && zones.covered.some((n) => n > 0) && (
        <p className="text-xs text-muted-foreground">
          Inside the spheres:{" "}
          <span className="text-foreground tabular-nums">
            {percent(zones.inside)}
          </span>{" "}
          solid
          {zones.unreinforced > 0 && (
            <>
              ,{" "}
              <span className="text-jade-hover tabular-nums">
                ×{(zones.inside / zones.unreinforced).toFixed(1)}
              </span>{" "}
              what they would hold without reinforcement
            </>
          )}
          .
        </p>
      )}
    </div>
  )
}

const AXES: Record<string, Point> = { x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1] }

/** A direction: none, one of the axes, or any other typed as dx, dy, dz. */
function DirectionField({ id, ...props }: FieldProps & { id: string }) {
  const value = props.value as Point | null
  const named =
    value === null
      ? "none"
      : (Object.keys(AXES).find((k) => sameValue(AXES[k], value)) ?? "custom")
  const [custom, setCustom] = React.useState(named === "custom")
  const mode = custom ? "custom" : named
  return (
    <div className="mb-[9px] flex flex-col gap-1.5">
      <Label {...props} htmlFor={mode === "custom" ? id : undefined} />
      <Segmented
        value={mode}
        options={[
          ["none", "None"],
          ["x", "X"],
          ["y", "Y"],
          ["z", "Z"],
          ["custom", "Custom"],
        ]}
        onChange={(m) => {
          if (m === "custom") {
            setCustom(true)
            if (!value) props.onChange([1, 1, 0])
            return
          }
          setCustom(false)
          props.onChange(m === "none" ? null : AXES[m])
        }}
      />
      {mode === "custom" && (
        <CommitInput
          id={id}
          value={value ? value.join(", ") : ""}
          placeholder="dx, dy, dz"
          onCommit={(s) => {
            const n = s
              .split(/[,;\s]+/)
              .filter(Boolean)
              .map(Number)
            if (
              n.length === 3 &&
              n.every(Number.isFinite) &&
              n.some((v) => v !== 0)
            )
              props.onChange(n)
          }}
        />
      )}
    </div>
  )
}

const FACE_NAMES: Record<string, string> = {
  "+z": "Top (+Z)",
  "-z": "Bottom (−Z)",
  "+x": "+X",
  "-x": "−X",
  "+y": "+Y",
  "-y": "−Y",
}

function Faces(props: FieldProps) {
  const open = new Set(props.value as string[])
  const toggle = (face: string) => {
    const next = new Set(open)
    if (next.has(face)) next.delete(face)
    else next.add(face)
    props.onChange([...next])
  }
  return (
    <div className="mb-[9px] flex flex-col gap-1">
      <Label {...props} />
      <div className="grid grid-cols-4 gap-1">
        {Object.keys(FACE_NAMES).map((face) => (
          <Toggle
            key={face}
            variant="outline"
            size="sm"
            pressed={open.has(face)}
            onPressedChange={() => toggle(face)}
            className={face[1] === "z" ? "col-span-2" : ""}
          >
            {FACE_NAMES[face]}
          </Toggle>
        ))}
      </div>
    </div>
  )
}

/** A text box that reports its value on Enter or when it loses focus. */
function CommitInput({
  value,
  onCommit,
  ...props
}: Omit<React.ComponentProps<typeof Input>, "value" | "onChange"> & {
  value: string
  onCommit: (s: string) => void
}) {
  const [draft, setDraft] = React.useState<string | null>(null)
  const done = () => {
    if (draft !== null && draft !== value) onCommit(draft)
    setDraft(null)
  }
  return (
    <Input
      {...props}
      value={draft ?? value}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={done}
      onKeyDown={(e) => {
        if (e.key === "Enter") done()
        if (e.key === "Escape") setDraft(null)
      }}
      className={cn("h-7 px-1.5 tabular-nums select-text", props.className)}
    />
  )
}
