# MakerWorld listing kit

Everything the upload form asks for, besides the description (`README.md`). Entry for the
**Biomimicry Design Contest** (makerworld.com/en/contests/170): entries close **30 November
2026 (UTC)**, and MakerWorld asks for the upload at least 24 hours earlier.

## Judging, and where the listing answers it

| Criterion | Weight | Where it is covered |
|---|---|---|
| Relevance | 10 % | "How nature solved it" and "What this model takes from real bone": each feature mapped to the biology it copies. Infographic (model picture 4). |
| Completion | 15 % | Two plates with profiles, painted supports, print times and weights, full print instructions, no assembly. |
| Innovation | 25 % | Led with from the top: bone-infill, a free generator that grows a printable bone lattice inside any model, with printing rules and a layer-by-layer checker (−92 % plastic in mid-air). Section "The real project", the cover badge, model picture 2, the banner in the web app. |
| Functionality | 25 % | A teaching model and a support-free anatomy slice; the free web app applies the same lattice to any model. |
| Visual appeal | 25 % | Photos and a video of the real print (their own slots in the form, and a section of the description), Blender renders, the slice, the infographic and the turntable video. |

## Web app banner

The web app shows a slim banner above the panels: made for the Biomimicry Design Contest,
with a link (`ui/apps/bone-infill/src/contest-banner.tsx`). It points at the contest page
for now; once the model is published, set `LINK` to its MakerWorld page and `LINK_TEXT` to
"See it on MakerWorld", then push so Pages redeploys. Visitors can close it.

## Title

The form allows 50 characters.

> Femur with Real Bone Inside + Free Generator

44 characters. It keeps the femur as the hook for search and puts the tool, the new part, in
the title. Alternatives: "Real Bone Inside: Printable Femur + Free Generator" (50) or
"Femur with Real Bone Inside – Printable Lattice" (49, model only).

## Category and tags

- **Category:** Education → Biology (type 'biology' in the category search).
- **Tags:** biomimicry, bone, femur, trabecular, lattice, anatomy, biology, education,
  osteoporosis, skeleton, science, organic, lightweight, nosupports, BiomimicryDesign

## License

The femur model is your own and the generator is open source, so any license fits. A
Creative Commons license that allows remixing (for example CC BY 4.0) matches the open
generator and lets teachers share prints; the Standard Digital File License keeps
commercial use with you.

## Files and print profile

Upload one Bambu Studio project with two plates, published as the print profile:

| Plate | Source | Settings | Time and filament (P2S) |
|---|---|---|---|
| 1. Femur (Low, one side open) | `out/real/femur_bone_infill_low_open.3mf` (your project, the one in the photos) | 0.20 mm Standard, 2 walls, tree supports painted under the head, build plate only, auto brim | 4 h 12 min, 43 g |
| 2. Bone slice (Medium) | `out/slice/femur_slice_medium.3mf` | 0.20 mm Standard, no supports, no brim | about 2 h 23 min, 22 g |

To add the slice: open your project in Bambu Studio, add a plate, import
`out/slice/femur_slice_medium.3mf`, leave it as it lands (flat, open faces up and down), turn
supports off for that plate and save. Name the plates as in the table.

The slice comes from the original `models/femur_proximal.stl`: laid on its back (the
coronal plane horizontal: X, Y, Z → Y, Z, X), filled whole at Medium, then cut. Filling the
slab after cutting would leave the skin open wherever the bone's surface runs almost
parallel to the cut, and grow the lattice for the wrong direction.

```bash
# out/slice/femur_flat.stl: models/femur_proximal.stl with (x, y, z) -> (y, z, x)
bone-infill generate out/slice/femur_flat.stl --cell 6 --density 0.28 --shell 2 --skin-boost 0.6 --voxel 0.2 -o out/slice/femur_flat_medium.stl
bone-infill section out/slice/femur_flat_medium.stl --normal 0,0,1 --up 0,1,0 --thickness 12 --voxel 0.2 -o out/slice/femur_slice_medium.3mf
```

- **Profile name:** `P2S 0.4 · Femur + bone slice · PLA`
- **Profile description:**

  > Plate 1 is the femur at Low density: supports only under the femoral head (already
  > painted), brim on. Plate 2 is a 12 mm bone slice at Medium density: no supports at all.
  > The lattice inside needs no supports; infill setting does not matter.

Also attach the STL of each plate for other printers and slicers.

## Pictures and videos, slot by slot

The folders in `gallery/` follow the "Model Pictures & Videos" form, top to bottom. Photos are
exported without EXIF data: the originals in `out/real/` carry the phone's GPS position, so do
not upload those.

**Model Covers** (jpg/gif/png, up to 30 MB)

- *Web/App cover 4:3* (required): `1_cover_4x3.jpg`, the open femur on black with the title
  "Real bone inside" and the free web app badge. Made in the style of past winners: big
  title, the part filling the frame, readable at card size.
- *App cover 3:4* (shown larger in the app): `1_cover_3x4.jpg`, the same design upright.

The form asks for real print photos on the cover; past winners often use a titled render, but
if you prefer a photo, `covers/cover_a_backlit_print.jpg` is the real print against the light
with the same title. `covers/cover_c_outside_inside.jpg` is the third option.

**Model Videos** (up to 30 s, shown first on the model page)

- `2_video_backlit.mp4`: the real print turning against the light, 15 s, 4:3.

**Real life photos** (required, at least 3, covering 3 views; up to 16)

1. `3_real_photos/01_on_the_table.jpg`: the open side, at an angle.
2. `3_real_photos/02_against_the_light.jpg`: the closed side, lit from behind.
3. `3_real_photos/03_on_the_build_plate.jpg`: straight off the plate, with its tree supports.

That meets the minimum. One or two more views would make it safer: the closed side in normal
light, the femur from above, the femur in your hand for scale, and the slice once printed.

**Model Pictures** (4:3 recommended; up to 16)

1. `01_cut_open_render.jpg`: the cut-open femur, rendered.
2. `02_web_app.jpg`: the web app with the femur generated: the tool.
3. `03_outside_inside.jpg`: the same bone from its closed and its open side.
4. `04_biomimicry.jpg`: infographic, biology → model.
5. `05_slice_top.jpg`: the bone slice from above.
6. `06_slice_angle.jpg`: the bone slice at an angle.
7. `07_detail.jpg`: close-up of the lattice.
8. `08_printability.jpg`: the same layer without and with the printing rules.
9. `09_densities.jpg`: Low, Medium and High.
10. `10_healthy_osteoporotic.jpg`: dense and sparse lattice.
11. `11_dark.jpg`: the femur on black.

The description's animated GIF (`img/real_backlit.gif`, 480 px, 7 MB) comes from the same clip
as the video; if the editor refuses it, use `img/real_backlit.jpg`. For social posts,
`social/` holds the backlit clip in 16:9 and the render turntable.

## Photos to take of the real print

Done: the femur on a table, against the light (photo and video), and on the build plate.
Still to take:

- The femur in your hand, for scale.
- The slice held up against a window, light coming through the pores.
- Femur and slice side by side.
- The slice on the build plate right after printing (the "no supports" proof).

## Community post (Community & Social Media Spotlight Award)

Post in the MakerWorld Community with the tags **#BiomimicryDesign #Contests**, and link the
model.

> **How do you 3D print the inside of a bone?**
>
> Bones are light because they are not solid: a thin hard shell around a sponge of plates
> and struts that puts material only where the loads go. I wanted that inside a printed
> femur, but a bone-like sponge is a printer's nightmare: struts that lean too far or start
> in mid-air, deep inside the part where no support can reach.
>
> So I wrote a generator that grows a bone lattice for any shape and then teaches it to
> print: upright walls, arches under every ceiling, nothing floating, and a checker that
> slices every result layer by layer. Against the same lattice without those rules, it cuts
> the plastic printed in mid-air by 92 %.
>
> The result: a femur with real-looking bone inside (43 g, no supports inside), and a 12 mm
> bone slice that prints with no supports at all and that you can hold up to the light.
>
> The generator is free, open source and runs in your browser, so you can grow bone inside
> your own models: https://jtorregrosa.github.io/trabecular-bone-infill-fdm/
>
> [model link]
> #BiomimicryDesign #Contests

Attach `social/backlit_16x9.mp4` (the real print turning against the light, 15 s, 1080p),
the infographic and the photo on the table. The same clip suits Instagram as a reel.

## Instagram caption

> A femur with real bone inside. A solid shell around a spongy lattice, grown for this
> exact shape and taught to print with no supports inside. Plus a bone slice you can hold up
> to the light. Free on MakerWorld; the generator is open source and runs in your browser.
> #BiomimicryDesign #madeinmakerworld #3dprinting #biomimicry #anatomy #bambulab

## Checklist

- [x] Print the femur and photograph it.
- [ ] Print the slice and photograph it (list above).
- [ ] Add the slice as plate 2 in the Bambu Studio project and save.
- [ ] Upload by 28 November: title, description (copy it from the preview page), tags,
      category, license, gallery, profile, STLs.
- [ ] Fill in the slice photo placeholder at the end of the description, or delete it.
- [ ] Contest page → *Join the Contest* → *Compete* on the model.
- [ ] Community post and Instagram post with the tags.
