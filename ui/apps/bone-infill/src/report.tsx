import { cn } from "@workspace/ui/lib/utils"
import { ChevronRight } from "lucide-react"

import { Button } from "@workspace/ui/components/button"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@workspace/ui/components/collapsible"
import { type Json, fmt } from "@workspace/ui/studio/api"
import { InfoCard, InfoTip } from "@workspace/ui/studio/info-tip"
import type { Viewer } from "@workspace/ui/studio/viewer"

type Level = "good" | "fair" | "poor"

/**
 * A one-line reading of the printability check. The thresholds are a rule of
 * thumb, stated in the tooltip; every figure is under "All figures".
 */
function verdict(out: Json): [Level, string, string] {
  const g = out.report
  const c = out.printability
  const closed =
    g.topology.boundary + g.topology.non_manifold + g.topology.misoriented === 0
  const pct: number = c.unsupported_interior_pct
  const islands: number = c.interior_islands
  const area: number = c.interior_island_area
  const why =
    `${fmt(pct, 2)} % unsupported inside · ` +
    (islands === 0
      ? "no islands in mid-air"
      : `${islands} island${islands === 1 ? "" : "s"} in mid-air (${fmt(area)} mm²)`)
  if (!closed)
    return [
      "poor",
      "The surface is not closed",
      "Some slicers will refuse or repair it.",
    ]
  if (area <= 1 && pct < 0.5) return ["good", "Looks printable", why]
  if (area <= 10 && pct < 2) return ["fair", "Small issues", why]
  return ["poor", "Check before printing", why]
}

const LEVEL_STYLE: Record<Level, string> = {
  good: "text-jade-hover border-jade/35",
  fair: "text-warn border-warn/35",
  poor: "text-destructive border-destructive/35",
}

/** Weight, saving and the printability verdict. */
export function Headline({ out }: { out: Json | null }) {
  if (!out)
    return <p className="text-muted-foreground">Nothing generated yet.</p>
  const g = out.report
  const [level, title, why] = verdict(out)
  // Motion: the block settles in once, with the first result. Later results
  // update the figures in place (they come often); only a change of verdict
  // fades in, because that is the news worth noticing.
  return (
    <div className="flex animate-in flex-col gap-2 duration-200 ease-out-strong fade-in-0 slide-in-from-bottom-1 motion-reduce:animate-none">
      <div className="grid grid-cols-2 gap-2">
        <Figure label="Weight in PLA" value={`${fmt(g.weight_pla_g, 0)} g`} />
        <Figure
          label="Lighter than solid"
          value={`${fmt(100 * (1 - g.volume_fraction), 0)} %`}
        />
      </div>
      <div
        key={level}
        className={cn(
          "flex animate-in items-baseline gap-2 rounded-md border px-2.5 py-[7px] duration-200 ease-out-strong fade-in-0 motion-reduce:animate-none",
          LEVEL_STYLE[level]
        )}
      >
        <span
          aria-hidden="true"
          className="size-2 shrink-0 -translate-y-px rounded-full bg-current"
        />
        <div className="min-w-0 flex-1">
          <div>{title}</div>
          <div className="text-xs text-muted-foreground">{why}</div>
        </div>
        <span className="self-start text-foreground">
          <InfoTip label="About the verdict">
            <InfoCard
              title="Printability verdict"
              about="Measured by slicing the result at the check layer height and looking for what a printer cannot do without support."
              effects={[
                [
                  "Looks printable",
                  "Islands in mid-air under 1 mm² in all, and under 0.5 % of the inside unsupported.",
                ],
                ["Small issues", "Under 10 mm² of islands and under 2 %."],
                [
                  "Check",
                  "Anything more: section at the worst layer to see it.",
                ],
              ]}
            />
          </InfoTip>
        </span>
      </div>
    </div>
  )
}

function Figure({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md bg-field px-2.5 py-2">
      <div className="text-[11.5px] text-subtle">{label}</div>
      <div className="mt-px text-xl font-semibold tabular-nums">{value}</div>
    </div>
  )
}

/** Every figure of the report, and a jump to the worst layer. */
export function AllFigures({
  out,
  viewer,
  layerHeight,
}: {
  out: Json | null
  viewer: Viewer | null
  layerHeight: number
}) {
  return (
    <Collapsible>
      <CollapsibleTrigger className="group/trigger flex items-center gap-1.5 font-medium text-muted-foreground outline-none hover:text-foreground">
        <ChevronRight className="size-3.5 transition-transform group-data-[state=open]/trigger:rotate-90" />
        All figures
      </CollapsibleTrigger>
      <CollapsibleContent className="max-h-[40vh] overflow-y-auto pt-1.5">
        {out ? (
          <Figures out={out} viewer={viewer} layerHeight={layerHeight} />
        ) : (
          <p className="text-muted-foreground">Nothing generated yet.</p>
        )}
      </CollapsibleContent>
    </Collapsible>
  )
}

function Figures({
  out,
  viewer,
  layerHeight,
}: {
  out: Json
  viewer: Viewer | null
  layerHeight: number
}) {
  const g = out.report
  const c = out.printability
  const rows: [string, string][] = [
    [
      "Lattice density",
      `${fmt(100 * g.lattice_density)} % (target ${fmt(100 * g.params.density)} %)`,
    ],
    [
      "Mean wall",
      `${fmt(g.calibration.mean_wall, 2)} mm` +
        (g.calibration.wall_clamped ? " (at minimum)" : ""),
    ],
    [
      "Volume",
      `${fmt(g.volume / 1000)} cm³ (${fmt(100 * g.volume_fraction, 0)} % of solid)`,
    ],
    ["Weight in PLA", `${fmt(g.weight_pla_g, 0)} g`],
    ["Voxel", `${fmt(g.voxel, 3)} mm · ${fmt(g.voxels / 1e6)} M`],
    ["Triangles", g.triangles.toLocaleString("en-US")],
    [
      "Closed surface",
      g.topology.boundary + g.topology.non_manifold + g.topology.misoriented ===
      0
        ? "yes"
        : "NO",
    ],
    [
      "Unsupported inside",
      `${fmt(c.unsupported_interior_area)} mm² (${fmt(c.unsupported_interior_pct, 3)} %)`,
    ],
    [
      "  skin / lattice",
      `${fmt(c.unsupported_skin_area ?? 0)} / ${fmt(c.unsupported_lattice_area ?? 0)} mm²`,
    ],
    [
      "Islands in mid-air",
      `${c.interior_islands} (+${c.interior_specks} specks)`,
    ],
    [
      "Bridges",
      `${fmt(c.bridge_area)} mm², longest ${fmt(c.longest_bridge)} mm`,
    ],
    ["Vaults added", `${fmt(g.vaults / 1000, 2)} cm³`],
    [
      "Carved",
      `${fmt((g.removed_unsupported + g.removed_thin + g.removed_loose) / 1000, 2)} cm³`,
    ],
  ]
  // What the reinforced spheres did, next to the lattice density.
  const z = g.load_zones
  if (z) {
    rows.splice(
      1,
      0,
      [
        "  in spheres",
        `${fmt(100 * z.inside)} % (${fmt(100 * z.unreinforced)} % without)`,
      ],
      ["  elsewhere", `${fmt(100 * z.outside)} %`]
    )
  }
  const worst = c.worst_layers?.[0] as [number, number] | undefined
  return (
    <>
      <table className="w-full border-collapse select-text">
        <tbody>
          {rows.map(([k, v]) => (
            <tr key={k}>
              <td className="py-0.5 align-top whitespace-pre text-muted-foreground">
                {k}
              </td>
              <td className="py-0.5 text-right align-top tabular-nums">{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {worst && viewer && (
        <Button
          variant="outline"
          size="sm"
          className="mt-2 h-auto w-full py-1.5 whitespace-normal"
          onClick={() => {
            viewer.update({ mode: 1 })
            viewer.cutAt(2, worst[0] + layerHeight / 2)
            viewer.setView("top")
          }}
        >
          Section at the worst layer (z = {fmt(worst[0])} mm, {fmt(worst[1])}{" "}
          mm²)
        </Button>
      )}
    </>
  )
}

/** One line about a new result, compared with the one before it. */
export function summarize(
  out: Json,
  seconds: number,
  previous: Json | null
): string {
  const now = {
    volume: out.report.volume / 1000,
    wall: out.report.calibration.mean_wall,
  }
  let text = `Done in ${fmt(seconds, 2)} s · ${fmt(now.volume)} cm³, walls ${fmt(now.wall, 2)} mm`
  if (previous) {
    const dv = now.volume - previous.report.volume / 1000
    const dw = now.wall - previous.report.calibration.mean_wall
    const sign = (v: number, d: number) =>
      (v >= 0 ? "+" : "−") + fmt(Math.abs(v), d)
    text +=
      Math.abs(dv) < 0.05 && Math.abs(dw) < 0.005
        ? " (same as before)"
        : ` (${sign(dv, 1)} cm³, ${sign(dw, 2)} mm)`
  }
  return text
}
