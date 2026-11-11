//! Description of the parameters for user interfaces.
//!
//! The web app builds its form from this (`GET /api/schema`), so the page
//! holds no knowledge of this generator: a different engine behind the same
//! API gets its own form for free. Each item names a field of [`Params`]
//! (`"perforation.0"` is an element of an array field), or a field of the
//! generate request itself when `request` is set.
//!
//! Item kinds: `number` (the default; a slider and a box, shown as
//! `value * scale`), `bool`, `seed`, `faces`, `loads`, `vec3` and `optional`
//! (a number that may be left empty for an automatic value).
//!
//! Every item explains itself for the info card next to its name: `about`
//! (what it is), `effects` (what raising or lowering it does, as
//! `[label, text]` pairs) and an optional practical `tip`.
//!
//! `presets` are named sets of values applied on top of the current ones.
//!
//! [`Params`]: crate::params::Params

use serde_json::{Value, json};

/// The product shown in the page header.
pub const PRODUCT_NAME: &str = "bone-infill";
pub const PRODUCT_TAGLINE: &str = "Trabecular-bone infill for FDM printing";

/// The schema served to the user interface.
pub fn schema() -> Value {
    json!({
        "product": { "name": PRODUCT_NAME, "tagline": PRODUCT_TAGLINE },
        "presets": presets(),
        "groups": [
            { "title": "Lattice", "open": true, "items": [
                { "key": "seed", "label": "Pattern (seed)", "kind": "seed",
                  "about": "Where the walls go. Each number gives a different, equally valid layout. The other parameters reshape that same layout, so their effect is easy to compare.",
                  "tip": "The circular arrow rolls a new layout; use it when a wall lands where you would rather not have it." },
                { "key": "open", "label": "Open faces", "kind": "faces",
                  "about": "Sides of the part left without skin, so the lattice shows and is exported that way.",
                  "tip": "For a slab printed lying flat, open Top and Bottom to show the bone on both faces." },
                { "key": "shell", "label": "Skin (mm)", "min": 0.4, "max": 8, "step": 0.1,
                  "about": "Thickness of the solid wall that wraps the lattice.",
                  "effects": [
                      ["Higher", "Stiffer part and a cleaner surface; more plastic and print time."],
                      ["Lower", "Lighter; with too few wall lines the lattice can show through the surface."]],
                  "tip": "Use a whole number of your slicer's wall lines: 2 mm is about 5 lines of 0.42 mm." },
                { "key": "cell", "label": "Cell size (mm)", "min": 3, "max": 40, "step": 0.5,
                  "about": "Typical spacing between walls: how big the pores are.",
                  "effects": [
                      ["Higher", "Bigger pores and fewer, thicker walls; faster to print, but wider ceilings that need more vaults."],
                      ["Lower", "A finer bone look and more even strength; more walls, longer prints, and walls that thin down to the minimum."]] },
                { "key": "density", "label": "Density (%)", "min": 5, "max": 50, "step": 0.5, "scale": 100,
                  "about": "Share of the inside filled with lattice material.",
                  "effects": [
                      ["Higher", "Stronger, stiffer and heavier, with thicker walls."],
                      ["Lower", "Lighter and faster. Walls stop thinning at the minimum wall, so very low values end up denser than asked."]],
                  "tip": "It is a volume fraction, like slicer infill: 15 % here weighs about as much as 15 % infill." },
                { "key": "z_elongation", "label": "Z elongation", "min": 1, "max": 4, "step": 0.1,
                  "about": "Stretches the pores along the print direction, so walls stand more upright.",
                  "effects": [
                      ["Higher", "Fewer overhangs and less trimming, so it prints more reliably; stiffer vertically than sideways."],
                      ["Lower", "Rounder pores, closer to real bone; more walls pass the overhang limit and get trimmed."]] },
            ] },
            { "title": "Printability", "items": [
                { "key": "max_overhang_deg", "label": "Max overhang (°)", "min": 20, "max": 70, "step": 1,
                  "about": "Steepest surface your printer can print without support, measured from vertical. Steeper lattice is trimmed or filled.",
                  "effects": [
                      ["Higher", "Keeps more of the natural lattice; needs good part cooling."],
                      ["Lower", "Safer on any printer; more material is trimmed or filled."]],
                  "tip": "45° is a safe value for most printers." },
                { "key": "max_bridge", "label": "Max bridge (mm)", "min": 0, "max": 12, "step": 0.5,
                  "about": "Longest horizontal span your printer bridges cleanly between two supported points.",
                  "effects": [
                      ["Higher", "Flat inner ceilings are accepted as they are: fewer vaults, less material."],
                      ["Lower", "More vaults and fills under ceilings, more material."]],
                  "tip": "Print a bridge test in your slicer and use the longest span that comes out clean." },
                { "key": "vaults", "label": "Vaults under ceilings", "kind": "bool",
                  "about": "Closes the pores under inner ceilings into pointed arches, so the skin above has something to rest on.",
                  "effects": [
                      ["On", "Inner ceilings print, at the cost of a little material."],
                      ["Off", "Lighter, but inner ceilings may sag or fail."]] },
                { "key": "vault_slot", "label": "Vault slot", "min": 0, "max": 1, "step": 0.05,
                  "about": "Width of the slot left open along the top of a vault, as a share of the max bridge.",
                  "effects": [
                      ["Higher", "Less material; the skin above bridges wider gaps."],
                      ["Lower", "Roofs closed more fully, more material."]] },
                { "key": "min_wall", "label": "Min wall (mm)", "min": 0.3, "max": 3, "step": 0.1,
                  "about": "Thinnest wall allowed. Thinner walls are thickened even if that exceeds the target density.",
                  "effects": [
                      ["Higher", "A sturdier lattice; heavier."],
                      ["Lower", "A finer lattice; below two line widths the slicer may drop walls."]],
                  "tip": "Use at least two line widths: 0.8 mm for a 0.4 mm nozzle." },
                { "key": "min_feature", "label": "Min feature (mm)", "min": 0, "max": 2, "step": 0.1,
                  "about": "Lattice features thinner than this are removed, since a nozzle cannot print them reliably.",
                  "effects": [
                      ["Higher", "A cleaner print with fewer stray strands."],
                      ["Lower", "Keeps more detail; 0 turns the removal off."]] },
                { "key": "min_bridge_width", "label": "Min bridge width (mm)", "min": 0.3, "max": 3, "step": 0.1,
                  "about": "Narrowest band of strands trusted as a bridge. Narrower spans are treated as overhangs.",
                  "effects": [
                      ["Higher", "Stricter: more material is trimmed or vaulted."],
                      ["Lower", "Accepts thinner bridges, which may sag."]] },
            ] },
            { "title": "Shape", "items": [
                { "key": "hole_ratio", "label": "Hole size (× cell)", "min": 0.2, "max": 1, "step": 0.05,
                  "about": "Size of the perforations through the walls, relative to the cell size.",
                  "effects": [
                      ["Higher", "Fewer, larger holes."],
                      ["Lower", "More, smaller holes: a more porous look."]] },
                { "key": "hole_elongation", "label": "Hole elongation", "min": 1, "max": 4, "step": 0.1,
                  "about": "How much taller than wide the holes through the walls are. 1 makes them round, like the perforations of real bone plates.",
                  "effects": [
                      ["Higher", "Slots that run up the walls; their tops close at a steeper angle and print a little more cleanly."],
                      ["Lower", "Round holes; their tops are small arches the printer bridges easily."]] },
                { "key": "perforation.0", "label": "Perforation, sparse", "min": 0, "max": 0.9, "step": 0.01,
                  "about": "How much of each wall is cut away where the lattice is sparsest.",
                  "effects": [
                      ["Higher", "Rod-like struts, like light spongy bone."],
                      ["Lower", "Plate-like walls; stiffer."]] },
                { "key": "perforation.1", "label": "Perforation, dense", "min": 0, "max": 0.9, "step": 0.01,
                  "about": "How much of each wall is cut away where the lattice is densest.",
                  "effects": [
                      ["Higher", "Open even where the lattice is dense."],
                      ["Lower", "Solid plates where the lattice is dense."]] },
                { "key": "bandwidth", "label": "Irregularity", "min": 0, "max": 1, "step": 0.05,
                  "about": "How varied the pore sizes are.",
                  "effects": [
                      ["Higher", "A more natural, random bone look."],
                      ["Lower", "Regular and almost periodic, closer to a classic infill."]] },
                { "key": "skin_boost", "label": "Density near skin", "min": 0, "max": 2, "step": 0.1,
                  "about": "Extra density where the lattice meets the skin, like the transition from compact to spongy bone.",
                  "effects": [
                      ["Higher", "A stronger bond between skin and lattice, and more support under the surface."],
                      ["Lower", "Even density throughout; lighter."]] },
                { "key": "blend", "label": "Skin fillet (mm)", "min": 0, "max": 4, "step": 0.1,
                  "about": "Radius of the fillet where walls join the skin.",
                  "effects": [
                      ["Higher", "Smoother joints with fewer stress points."],
                      ["Lower", "Sharper joints; slightly less material."]] },
            ] },
            { "title": "Resolution", "items": [
                { "key": "voxel", "label": "Voxel (mm)", "min": 0.15, "max": 1, "step": 0.05,
                  "about": "Resolution of the computation.",
                  "effects": [
                      ["Higher", "Faster and lighter on memory; coarser walls."],
                      ["Lower", "Smoother surfaces and thinner walls possible. Halving it takes about 8 times the time and memory."]] },
                { "key": "max_voxels", "label": "Voxel budget (M)", "min": 5, "max": 200, "step": 5, "scale": 1e-6,
                  "about": "Most voxels, in millions, a part may use. A larger part gets a coarser voxel automatically.",
                  "effects": [
                      ["Higher", "Keeps detail on large parts; needs more memory."],
                      ["Lower", "Faster, and safer on machines with little memory."]] },
                { "key": "waves", "label": "Waves per field", "min": 16, "max": 256, "step": 8, "integer": true,
                  "about": "Number of wave components that build each random field of the pattern.",
                  "effects": [
                      ["Higher", "A more random-looking pattern; slower."],
                      ["Lower", "Faster; the pattern may start to repeat visibly."]] },
                { "key": "simplify", "label": "Simplify (mm)", "kind": "optional", "placeholder": "auto",
                  "about": "How far the surface may move when triangles are merged. Empty for automatic (a quarter of a voxel), 0 to keep every triangle.",
                  "effects": [
                      ["Higher", "A smaller file."],
                      ["Lower", "A more faithful surface and a bigger file."]] },
                { "key": "compensate_density", "label": "Compensate density", "kind": "bool",
                  "about": "The printability passes remove some material. With this on, the part is generated again with a corrected target so that the result has the density asked for.",
                  "effects": [
                      ["On", "The density asked for; a little slower."],
                      ["Off", "Faster; the part can come out lighter than the target."]] },
                { "key": "layer_height", "label": "Check layer (mm)", "min": 0.05, "max": 0.6, "step": 0.01,
                  "request": true, "default": 0.2,
                  "about": "Layer height used by the printability check. It changes how the part is judged, not the part.",
                  "tip": "Use the layer height you will slice with." },
            ] },
        ],
    })
}

/// Three levels of material, from light to strong. Each sets the same
/// parameters, so moving between them leaves nothing behind; every other
/// parameter keeps its current value.
fn presets() -> Value {
    json!([
        { "name": "Low", "about": "Light: display pieces and quick prints.",
          "params": { "density": 0.20, "cell": 8.0, "shell": 1.6, "skin_boost": 0.4 } },
        { "name": "Medium", "about": "Balanced: the defaults.",
          "params": { "density": 0.28, "cell": 6.0, "shell": 2.0, "skin_boost": 0.6 } },
        { "name": "High", "about": "Strong: parts that carry load.",
          "params": { "density": 0.36, "cell": 4.5, "shell": 2.8, "skin_boost": 0.9 } },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Params;

    #[test]
    fn every_item_names_a_parameter_and_explains_itself() {
        let defaults = json!(Params::default());
        let schema = schema();
        let mut count = 0;
        for group in schema["groups"].as_array().unwrap() {
            for item in group["items"].as_array().unwrap() {
                let key = item["key"].as_str().unwrap();
                count += 1;
                assert!(item["about"].as_str().is_some_and(|s| s.len() > 20), "{key} needs an explanation");
                if let Some(effects) = item.get("effects") {
                    for effect in effects.as_array().unwrap() {
                        assert_eq!(effect.as_array().unwrap().len(), 2, "{key}: effects are [label, text]");
                    }
                }
                if item["request"] == true {
                    assert!(item["default"].is_number(), "{key}: a request field needs a default");
                    continue;
                }
                let (field, index) = key.split_once('.').map_or((key, None), |(f, i)| (f, Some(i)));
                let value = &defaults[field];
                assert!(!value.is_null() || field == "simplify" || field == "stretch_dir", "{key} is not a parameter");
                if let Some(i) = index {
                    assert!(value[i.parse::<usize>().unwrap()].is_number(), "{key} is not an array element");
                }
                if item.get("kind").is_none() {
                    let (min, max) = (item["min"].as_f64().unwrap(), item["max"].as_f64().unwrap());
                    let scale = item["scale"].as_f64().unwrap_or(1.0);
                    let shown = match index {
                        Some(i) => value[i.parse::<usize>().unwrap()].as_f64(),
                        None => value.as_f64(),
                    };
                    let shown = shown.unwrap() * scale;
                    assert!(min <= shown && shown <= max, "{key}: default {shown} outside {min}..{max}");
                }
            }
        }
        assert!(count > 20);
    }

    #[test]
    fn presets_are_valid_and_medium_is_the_default() {
        let presets = presets();
        let presets = presets.as_array().unwrap();
        assert_eq!(presets.len(), 3);
        let defaults = json!(Params::default());
        let keys: Vec<_> = presets[0]["params"].as_object().unwrap().keys().cloned().collect();
        for preset in presets {
            let values = preset["params"].as_object().unwrap();
            // Same parameters in each, so switching presets leaves nothing behind.
            assert_eq!(values.keys().cloned().collect::<Vec<_>>(), keys);
            let mut merged = defaults.clone();
            for (k, v) in values {
                assert!(!defaults[k].is_null(), "{k} is not a parameter");
                merged[k] = v.clone();
            }
            let params: Params = serde_json::from_value(merged).unwrap();
            params.validate().unwrap();
            if preset["name"] == "Medium" {
                for (k, v) in values {
                    assert_eq!(&defaults[k], v, "Medium must equal the defaults ({k})");
                }
            }
        }
    }
}
