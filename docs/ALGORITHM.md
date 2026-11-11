# How bone-infill works, and why

This document follows the pipeline in order. Each section says what the step does, why it
is done that way, and, where relevant, what the Python prototype did instead.

All fields live on a regular voxel grid (0.3 mm by default) stored X-fastest, so one Z layer
is one contiguous slice. Layer-wise steps run in parallel over those slices.

## 1. What was wrong in the prototype

Two findings from reading the prototype drove the redesign. Both were confirmed by an
independent derivation and then by measurement.

**The anisotropy was inverted.** The prototype divided the Z component of the wave vectors
by `z_stretch = 0.6`. That raises the frequency along Z, which shortens features vertically
and turns sheets towards *horizontal*, the opposite of what the comment said. For a Gaussian
field the share of sheet area that faces down beyond 45° is `(1 - 1/sqrt(1 + rho^2)) / 2`,
with `rho` the ratio of vertical to horizontal gradient: about 28 % as written, 15 % for an
isotropic field, 5 % with the intended elongation. The same inversion applied to the in-plane
`stretch`.

**The support filter allowed about 80°.** Each layer could grow one voxel beyond the layer
below, plus a 1.6 mm "bridge" that was really a cantilever (grown from one side only), and
that compounded layer after layer: 1.8 mm sideways per 0.3 mm up. It also treated every skin
voxel as supported, so inner ceilings were never looked at.

Measured on the prototype's bunny, 45 % of the inner down-facing surface is steeper than 45°.

## 2. Voxelisation and distances

`voxelize` casts one ray per grid row along X. Triangles are rasterised on the YZ plane with
half-open rules (edges are evaluated from their lower endpoint, intervals are `[a, b)`), so a
ray through a shared edge or vertex is counted exactly once. Crossings carry the sign of the
facet normal and rows are filled by non-zero winding, which handles cavities and overlapping
shells.

`edt` is an exact Euclidean distance transform (Felzenszwalb and Huttenlocher), separable and
parallel. The Z pass copies each XZ slab to a compact buffer so the columns are cache
friendly. Half a voxel is subtracted from the result so that `--shell` is the real thickness.

`--open` faces work as in the prototype: outside voxels lying beyond the model along that
direction are treated as interior for the distance, so no skin forms on surfaces facing it.

## 3. The bone field

The field is a number per voxel, in millimetres, positive where there is material.

**Random fields.** `g(p) = sqrt(2/N) * sum cos(k_i . p + phi_i)` has zero mean and unit
variance by construction and can be evaluated anywhere, so each row is computed on its own.
Per-axis `cos`/`sin` tables reduce a row to two multiply-adds per wave and voxel; the kernel
is compiled for AVX2 and for the baseline and chosen at run time. Results are bit-identical
on both paths and for a given seed (the random generator is implemented in the crate).

**Anisotropy.** To make features `e` times longer along an axis, the wave-vector component
along it is divided by `e`. `--z-elongation 2` halves the vertical gradient, which is what
makes most sheets steep.

**Sheets of uniform thickness.** The prototype used `|g1| < w`, whose thickness varies with
the local gradient. Here the sheet is `|g1| / |grad g1| < t/2`: dividing by the analytic
gradient turns the field into a distance to the mid-surface, so walls have a real thickness
`t` and `--min-wall` can be enforced. The gradient costs three more sums per voxel.

**Perforation.** A second, finer field removes the part of the sheets where `g2` is below a
threshold. Where material is scarce the remaining ligaments are the rods.

**Density.** A demand map (higher near the skin and in `--load` spheres) is normalised to its
5–95 % range over the interior. Wall thickness and perforation both follow it. One global
offset is found by bisection on 200 000 sampled voxels so the lattice hits `--density`.

**Skin.** The lattice is merged with the skin by a smooth maximum, which leaves fillets at
every junction. A light binomial blur removes the terracing that a voxel distance field
leaves on the inner side of the skin.

## 4. The overhang filter (`support`)

Layers are visited from the bed up. A lattice voxel is kept if:

- **Overhang rule:** it is within `tan(max_overhang)` layer heights of material kept in the
  layer below; or
- **Bridge rule:** it belongs to a bridge (below).

Otherwise it is carved. The skin is never touched.

The rule is evaluated on distances, not on a mask. Each layer records `a(q)`, how deep voxel
`q` sits inside the kept material (read from the field). A voxel `p` of the next layer may
extend to

```
allowance(p) = tan(max_overhang) + max_q ( a(q) - |p - q| )
```

and the field is clipped to it. This is the in-plane signed distance to the previous outline
grown by one layer of overhang, so the limit is the same in every direction (a 3×3 mask gives
45° along the axes and 55° on diagonals) and the cut is a smooth plane. The empty voxel under
a clipped one is lowered to the matching distance; otherwise the surface between the two
layers would be interpolated against the original field and hang down as a small lip.

The consequence for shapes: a hole in a wall keeps its lower half and gets a pointed roof,
a horizontal plate between two walls loses its middle and keeps two corbels, and anything
attached only from above disappears.

After it, features thinner than `--min-feature` are removed (an opening that reads radii from
the field itself), the filter runs again in case something lost its footing, and lattice not
connected to the skin through faces is removed.

Because carving removes material, the field is regenerated with a corrected target (secant
iteration, at most four passes) until the lattice density lands within 3 % of `--density`.

## 5. Bridges (`bridge`)

A bridge is a band of parallel strands, each resting on supported material at both ends. An
unsupported pixel is accepted when:

1. it lies on a straight run of unsupported pixels, along one of four directions, that
   touches supported material at both ends and is no longer than `--max-bridge`; and
2. across that direction, equally short strands or supported material continue for at least
   0.8 mm.

The second condition rejects the stray one-pixel chords that can always be drawn across the
corner of a ledge. Two earlier versions were tried and measured: accepting pixels one by one
left unprintable hairs around carved holes, and accepting only regions bridgeable in a single
direction rejected whole slot networks for one wide junction.

When generating, a connected unsupported region is kept only if all of it bridges, and with a
10 % margin on the length; a half-bridged region would leave a ragged edge. When checking,
each pixel is judged on its own.

## 6. Vaults (`vault`)

A closed part has a roof. Its first layer would have to span whole pores, or, under a sloping
roof, stick out further at every layer.

The condition for a cavity to be printable is about its empty space: going down from a roof,
a pore may only widen by `tan(overhang)` per layer. So empty space has to descend from
somewhere. The pass sweeps from the top down keeping `r`, the signed distance to the region
allowed to be empty:

1. each layer down, the region grows by `tan(overhang)`;
2. skin, the outside, and anything this pass has filled are roof;
3. a pore found deep under roof, with no empty space reaching it from above, is seeded with
   a slot along its medial axis: half a bridge wide under a flat roof, a hairline under a
   sloping one where nothing can be bridged;
4. what is still under roof is filled.

The result is a pointed vault springing from the walls of each pore. Under a dome it is the
wedge between the dome and a cone of the printable angle, and it vanishes where the dome is
already steep enough. The medial axis comes from a feature transform (nearest wall point per
pixel): a pixel is on the axis when it and a neighbour see wall points subtending more than
80°. An angle, not a distance, so that a large round pore gives a point and not a blob.

Vault material is left out of the density target; it is reported separately.

## 7. Meshing

**Marching cubes.** The 256-case table is generated at start-up: for each sign pattern the
surface is traced on the six cube faces, one directed segment per run of positive corners,
and the segments close into loops. The pairing on a face depends only on that face's four
corner signs, so two cubes sharing a face always agree, and the mesh is closed. Loops are
triangulated avoiding diagonals that lie in a cube face, because a neighbour could pick the
same diagonal and four triangles would share an edge. Tests extract white noise and require
every edge to have exactly two triangles.

Vertices are created per voxel layer on the grid edges they own, then triangles per layer of
cells look them up, both in parallel, so vertices are shared exactly with no welding.

**Closed skin.** Outside the model the field is positive, so the extracted surface is only
the cavities. The input mesh is prepended unchanged: the outer surface is exact and no mesh
boolean is needed.

**Open faces.** The field is intersected with an exact narrow-band distance to the input
triangles and the whole part is extracted. Flat faces come out flat; sharp edges are rounded
at voxel scale.

**Simplification.** Quadric edge collapse, cheapest first, while the merged vertex stays
within the tolerance of the planes of all triangles it stands for. A link-condition test
keeps the mesh a closed manifold and a normal test prevents flips. Large meshes are cut into
runs of consecutive triangles, simplified in parallel with their seam vertices frozen.

## 8. The check (`analyze`)

The check shares the bridge rule with the generator but not its grid: the finished mesh is
sliced at the print layer height on a finer XY raster. A pixel is supported (material
directly below), a gentle overhang (material below within the limit), a bridge, or
unsupported. With a reference model, unsupported pixels the original also leaves unsupported
are reported as the model's own; with the skin thickness, the rest is split into skin and
lattice by depth inside the reference. Regions with no anchored pixel are islands.

It is a geometric model of a slicer, not a slicer. Its main simplifications are the four
bridge directions, a pixel raster, and no notion of extrusion width, cooling or speed.

## 9. Things that were tried and replaced

Kept here because the dead ends explain the current design.

- **Flaring the lattice by a fixed depth under ceilings.** Cheap, but closes only pores up
  to a chosen size.
- **Vaults from a medial axis recomputed independently in each layer.** When a wall ends or
  starts, the axis jumps and material appears with nothing under it.
- **Measuring roof clearance above each point rather than at the pore axis.** Fills a wide
  flat ledge the moment a wall starts to lean.
- **Medial axis from ridges of the distance map.** The voxel staircase of every diagonal
  wall produces false ridges.
- **Propagating support with a binary mask.** Direction-dependent limit and stepped cuts.

## 10. Where to go next

- Print a test coupon cut open and compare with the report; calibrate the bridge and
  overhang limits per material.
- Run a slicer from the command line on the output and count its overhang and bridge
  extrusions, as a check that does not share any code with the generator.
- Reduce what is left under ceilings: narrower slots where they meet, or a slot direction
  chosen per roof.
- Drive the density map from a stress field instead of a heuristic.
- Sharp-feature preservation when re-meshing open parts.
