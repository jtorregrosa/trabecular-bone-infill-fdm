import { type Preset, type Values, sameValue } from "./api"
import { InfoTip } from "./info-tip"
import { Segmented } from "./view-controls"

/** The preset whose values all match the current parameters, if any. */
export function activePreset(presets: Preset[], params: Values) {
  return presets.find((preset) =>
    Object.entries(preset.params).every(([k, v]) => sameValue(params[k], v))
  )
}

/**
 * Quick choice between the schema's presets. When the parameters were changed
 * by hand so that no preset matches, none is selected and "Custom" shows.
 */
export function PresetPicker({
  presets,
  params,
  onPick,
}: {
  presets: Preset[]
  params: Values
  onPick: (preset: Preset) => void
}) {
  if (presets.length === 0) return null
  const active = activePreset(presets, params)
  return (
    <div className="mb-3">
      <div className="mb-1.5 flex items-center gap-1.5">
        <span className="font-semibold">Preset</span>
        <InfoTip label="About the presets">
          <div className="flex flex-col gap-2">
            <div className="font-semibold">Presets</div>
            <dl className="grid grid-cols-[auto_1fr] gap-x-2 gap-y-1">
              {presets.map((p) => (
                <div key={p.name} className="contents">
                  <dt className="font-medium text-jade-hover">{p.name}</dt>
                  <dd>{p.about}</dd>
                </div>
              ))}
            </dl>
            <p className="border-t border-line pt-2 text-muted-foreground">
              A preset sets density, cell size, skin and the density near the
              skin. Everything else keeps its value.
            </p>
          </div>
        </InfoTip>
        <span
          className={
            "ml-auto text-xs text-subtle transition-opacity duration-150 motion-reduce:transition-none " +
            (active ? "opacity-0" : "opacity-100")
          }
        >
          Custom
        </span>
      </div>
      <Segmented
        value={active?.name ?? ""}
        options={presets.map((p) => [p.name, p.name] as [string, string])}
        onChange={(name) => {
          const preset = presets.find((p) => p.name === name)
          if (preset) onPick(preset)
        }}
      />
    </div>
  )
}
