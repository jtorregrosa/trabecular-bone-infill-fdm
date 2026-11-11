# Design system

One set of design tokens, written in two places that must agree:

- [`ui/packages/ui/src/styles/globals.css`](../ui/packages/ui/src/styles/globals.css): the
  studio kit, used by the web app (`bone-infill serve` and its WebAssembly build). The tokens
  are also mapped onto shadcn/ui's roles there (`--primary` is jade, `--card` the panel
  surface, and so on), so its components follow them.
- [`src/web/theme.css`](../src/web/theme.css): the standalone viewer (`bone-infill view`), a
  plain HTML page the crate writes itself.

The PNG preview (`src/render.rs`) and the WebGL shader
(`ui/packages/ui/src/studio/viewer-core.js`) repeat the colours of the part, marked in
their source. The renderer reads `--canvas` from the page for its background.

## Idea

Three roles, each with its own colour family, never mixed:

| Role | Family | Used for |
|---|---|---|
| Interface | Cool neutral grays | Panels, controls, text |
| Interaction | Jade | The primary action, selected state, focus, progress |
| The part | Bone | The model itself and its cut faces |

The grays follow the dark theme of Bambu Studio, so the app sits naturally next to the slicer
and MakerWorld. The green is our own: a jade, bluer and softer than Bambu Lab's brand green
(`#00AE42`), so the tool reads as part of that world without passing for an official product.

## Brand

The logo, the icons and the rules for using them are in [the brand pack](brand/README.md). The
app takes them from there, never from a copy:

- The header and the empty state draw `docs/brand/svg/logo-currentcolor.svg` in bone
  (`ui/apps/bone-infill/src/logo.tsx`, through the `@brand` alias).
- The favicon and the loading screen are `icon.svg` and `logo-bone.svg`, written into
  `index.html` as data URIs at build time (the `%BRAND:<file>%` placeholders, replaced in
  `vite.config.ts`).
- The standalone viewer embeds `icon.svg` as its favicon (`src/viewer.rs`).

## Tokens

### Grays

| Token | Value | Use |
|---|---|---|
| `--canvas` | `#1b1b1f` | Behind the part in the 3D view |
| `--surface` | `#242428` | Side panels |
| `--field` | `#2d2d31` | Buttons, inputs |
| `--raised` | `#36363c` | Hover on fields |
| `--line` | `#3e3e45` | Hairlines, resting borders |
| `--line-strong` | `#54545b` | Emphasised borders, hover |
| `--text` | `#efeff0` | Body text |
| `--text-2` | `#b3b3b4` | Supporting text (`.dim`) |
| `--text-3` | `#818183` | Hints, captions, section headings |

### Jade

| Token | Value | Use |
|---|---|---|
| `--jade` | `#3dbe8b` | Primary button, focus ring, slider, spinner |
| `--jade-hover` | `#56cd9d` | Hover; text of a selected segment |
| `--jade-pressed` | `#2fa275` | Pressed primary button |
| `--jade-tint` | jade at 12 % | Selected segment, drop target, text selection |
| `--on-jade` | `#0d1c16` | Text on a jade fill |

### Bone

| Token | Value | Use |
|---|---|---|
| `--bone` | `#ede6d6` | The part, its cut faces (same colour, softer light) and the logo mark |

### Status and overhang scale

`--danger` `#e5675c` and `--warn` `#e9a23b` for messages. The overhang colouring uses
`--oh-safe` `#7fb48c` (under 30°), `--oh-mild` `#d9c860` (30–45°), `--oh-steep` `#e08a3c`
(45–60°) and `--oh-bad` `#d6483b` (over 60°), with the `.oh-*` classes for the legend.

### Shape and type

- Radius: `--radius` 6 px for controls, `--radius-lg` 10 px for panels and the drop zone,
  fully round for the progress toast.
- Type: the system UI font at 13 px (`--font`), monospace at 11 px (`--mono`). Section
  headings are 11 px uppercase with letter spacing, in `--text-3`.
- Motion: 120 ms colour transitions on `--ease`. Nothing moves on screen except the spinner.

### Motion

A tool used for long sessions, so motion is there to inform, not to decorate:

- Under 300 ms, on `--ease-out-strong` (`cubic-bezier(0.23, 1, 0.32, 1)`); exits are shorter
  and plainer than entrances.
- Animated: things that appear rarely or carry news. The loading screen (only if loading
  takes over 150 ms), the empty state, the first result, a change of printability verdict,
  the status toast, the canvas fading in with a new model, groups opening, info cards, and
  the changed-parameter dot.
- Not animated: what happens many times a session or comes from the keyboard. Figures of
  each new result, view changes from shortcuts, slider values.
- Only `opacity`, `transform` and `filter` move. Every animation has a reduced-motion path,
  with a global safety net in `globals.css`; the busy spinner is the one exception and turns
  slowly.

## Rules

1. One jade fill per view: the Generate button. Every other selected state uses the tint.
2. Bone is only for the part, and for the logo. Controls never use it, so the model always
   stands out.
3. Pages stay self-contained: no web fonts, CDNs or image files. The app runs offline and from
   a single binary; the logo and the icon are inlined into the page when it is built.
4. A new colour becomes a token first (in both files above), then gets used.
5. Explanations stay out of the way: a control shows its name only, and what it does sits
   behind an info icon beside the name (`InfoTip`), opened by hover, focus or a tap.
6. shadcn/ui components are our code once added: adapt them to these rules rather than
   overriding them from outside (the toggles, for instance, show their pressed state with the
   jade tint).
