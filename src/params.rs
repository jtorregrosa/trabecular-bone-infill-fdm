//! User-facing generation parameters.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// A side of the model's bounding box, used to leave a face without skin.
/// Serialised as the same text the command line takes, e.g. `"+z"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Face {
    /// 0 = X, 1 = Y, 2 = Z.
    pub axis: usize,
    /// True for the `+` side.
    pub positive: bool,
}

impl std::str::FromStr for Face {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let b = s.trim().as_bytes();
        if b.len() != 2 {
            bail!("invalid face '{s}' (use +x -x +y -y +z -z)");
        }
        let positive = match b[0] {
            b'+' => true,
            b'-' => false,
            _ => bail!("invalid face '{s}' (use +x -x +y -y +z -z)"),
        };
        let axis = match b[1].to_ascii_lowercase() {
            b'x' => 0,
            b'y' => 1,
            b'z' => 2,
            _ => bail!("invalid face '{s}' (use +x -x +y -y +z -z)"),
        };
        Ok(Face { axis, positive })
    }
}

impl TryFrom<String> for Face {
    type Error = anyhow::Error;
    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<Face> for String {
    fn from(f: Face) -> String {
        format!("{}{}", if f.positive { '+' } else { '-' }, ['x', 'y', 'z'][f.axis])
    }
}

/// A spherical region where the lattice is made denser.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Load {
    pub center: [f64; 3],
    pub radius: f64,
}

/// Every field has a default, so a partial JSON object is a valid `Params`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    // ---- geometry of the bone ------------------------------------------------
    /// Thickness of the solid skin (mm).
    pub shell: f64,
    /// Cell size: wavelength of the sheets (mm).
    pub cell: f64,
    /// Perforation wavelength relative to `cell`.
    pub hole_ratio: f64,
    /// How much longer the perforations are along Z than across. 1 gives
    /// round holes; the sheets keep their own `z_elongation`.
    pub hole_elongation: f64,
    /// Share of each sheet that is perforated, where the lattice is sparsest
    /// and where it is densest. More perforation gives rods, less gives plates.
    pub perforation: [f64; 2],
    /// Relative spread of the wavelengths in the pattern: 0 is a regular,
    /// almost periodic lattice, higher is more irregular.
    pub bandwidth: f64,
    /// Target solid fraction of the lattice region.
    pub density: f64,
    /// Extra density next to the skin (0 = uniform).
    pub skin_boost: f64,
    /// Fillet radius where lattice and skin merge (mm).
    pub blend: f64,
    /// Pores are this many times longer along Z than across it. Values above
    /// 1 turn sheets towards vertical, which is what makes them printable.
    pub z_elongation: f64,
    /// Main load direction; pores are elongated along it.
    pub stretch_dir: Option<[f64; 3]>,
    /// Elongation factor along `stretch_dir`.
    pub stretch: f64,
    /// Minimum sheet thickness (mm). Thinner sheets are thickened even if
    /// that exceeds the target density.
    pub min_wall: f64,
    /// Faces left without skin so the lattice is visible.
    pub open: Vec<Face>,
    /// Reinforced regions.
    pub loads: Vec<Load>,
    /// How much denser the lattice gets inside a load region (0 = no effect).
    pub load_boost: f64,

    // ---- printability --------------------------------------------------------
    /// Steepest lattice overhang allowed, in degrees from vertical.
    pub max_overhang_deg: f64,
    /// Longest span (mm) that may be printed as a bridge resting on supported
    /// material at both ends.
    pub max_bridge: f64,
    /// Narrowest band of bridge strands accepted as a bridge (mm).
    pub min_bridge_width: f64,
    /// Close the pores under inner ceilings into pointed vaults so the skin
    /// above them has something to rest on.
    pub vaults: bool,
    /// Width of the slots the vaults leave open under a flat roof, as a share
    /// of `max_bridge`. Where slots meet the opening is wider than one slot,
    /// which is why the default is well below 1.
    pub vault_slot: f64,
    /// Lattice features thinner than this (mm) are removed. 0 disables.
    pub min_feature: f64,
    /// Regenerate with a corrected target when the printability passes remove
    /// enough material to miss `density`.
    pub compensate_density: bool,

    // ---- resolution ----------------------------------------------------------
    /// Voxel size (mm).
    pub voxel: f64,
    /// Voxel budget; the voxel is enlarged for parts that would exceed it.
    pub max_voxels: f64,
    /// Plane waves per noise field.
    pub waves: usize,
    /// Mesh simplification tolerance (mm). `None` = a quarter of a voxel,
    /// 0 disables simplification.
    pub simplify: Option<f64>,
    /// Pattern seed.
    pub seed: u64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            shell: 2.0,
            cell: 6.0,
            hole_ratio: 0.45,
            hole_elongation: 1.0,
            perforation: [0.39, 0.27],
            bandwidth: 0.35,
            density: 0.28,
            skin_boost: 0.6,
            blend: 1.2,
            z_elongation: 1.3,
            stretch_dir: None,
            stretch: 1.6,
            min_wall: 0.8,
            open: Vec::new(),
            loads: Vec::new(),
            load_boost: 0.8,
            max_overhang_deg: 45.0,
            max_bridge: 4.0,
            min_bridge_width: 0.8,
            vaults: true,
            vault_slot: 0.5,
            min_feature: 0.6,
            compensate_density: true,
            voxel: 0.3,
            max_voxels: 60.0e6,
            waves: 128,
            simplify: None,
            seed: 7,
        }
    }
}

impl Params {
    pub fn validate(&self) -> Result<()> {
        let positive = [
            ("shell", self.shell),
            ("cell", self.cell),
            ("hole-ratio", self.hole_ratio),
            ("hole-elongation", self.hole_elongation),
            ("voxel", self.voxel),
            ("max-voxels", self.max_voxels),
            ("z-elongation", self.z_elongation),
            ("stretch", self.stretch),
        ];
        for (name, v) in positive {
            if !(v.is_finite() && v > 0.0) {
                bail!("--{name} must be a positive number (got {v})");
            }
        }
        if !(0.02..=0.9).contains(&self.density) {
            bail!("--density must be between 0.02 and 0.9 (got {})", self.density);
        }
        if !(10.0..=85.0).contains(&self.max_overhang_deg) {
            bail!("--max-overhang must be between 10 and 85 degrees (got {})", self.max_overhang_deg);
        }
        if self.max_bridge < 0.0
            || self.blend < 0.0
            || self.min_wall < 0.0
            || self.min_feature < 0.0
            || self.min_bridge_width < 0.0
        {
            bail!("lengths cannot be negative");
        }
        if self.perforation.iter().any(|f| !(0.0..=0.9).contains(f)) {
            bail!("--perforation values must be between 0 and 0.9 (got {:?})", self.perforation);
        }
        if !(0.0..=1.0).contains(&self.bandwidth) {
            bail!("--bandwidth must be between 0 and 1 (got {})", self.bandwidth);
        }
        if !(0.0..=1.0).contains(&self.vault_slot) {
            bail!("--vault-slot must be between 0 and 1 (got {})", self.vault_slot);
        }
        if !(self.load_boost.is_finite() && self.load_boost >= 0.0) {
            bail!("--load-boost cannot be negative (got {})", self.load_boost);
        }
        if !(self.skin_boost.is_finite() && self.skin_boost >= 0.0) {
            bail!("--skin-boost cannot be negative (got {})", self.skin_boost);
        }
        if self.waves < 8 {
            bail!("--waves must be at least 8");
        }
        if self.cell < 6.0 * self.voxel {
            bail!(
                "--cell ({} mm) is too small for the voxel size ({} mm); use at least {} mm",
                self.cell,
                self.voxel,
                6.0 * self.voxel
            );
        }
        if let Some(d) = self.stretch_dir
            && d.iter().map(|v| v * v).sum::<f64>() <= 0.0
        {
            bail!("--stretch-dir cannot be the zero vector");
        }
        Ok(())
    }

    /// Simplification tolerance (mm) for a given voxel size.
    pub fn simplify_tolerance(&self, voxel: f64) -> f64 {
        self.simplify.unwrap_or(0.25 * voxel).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_parse() {
        assert_eq!("+z".parse::<Face>().unwrap(), Face { axis: 2, positive: true });
        assert_eq!("-X".parse::<Face>().unwrap(), Face { axis: 0, positive: false });
        assert!("z".parse::<Face>().is_err());
        assert!("+w".parse::<Face>().is_err());
    }

    #[test]
    fn json_round_trip_and_partial_objects() {
        let p = Params { open: vec![Face { axis: 2, positive: true }], cell: 9.0, ..Params::default() };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"open\":[\"+z\"]"), "{json}");
        let back: Params = serde_json::from_str(&json).unwrap();
        assert_eq!(back.open, p.open);
        assert_eq!(back.cell, 9.0);
        let partial: Params = serde_json::from_str(r#"{"density": 0.2, "open": ["-x"]}"#).unwrap();
        assert_eq!(partial.density, 0.2);
        assert_eq!(partial.shell, Params::default().shell);
        assert_eq!(partial.open, vec![Face { axis: 0, positive: false }]);
        assert!(serde_json::from_str::<Params>(r#"{"open": ["up"]}"#).is_err());
    }

    #[test]
    fn defaults_are_valid() {
        Params::default().validate().unwrap();
    }

    #[test]
    fn bad_values_are_rejected() {
        assert!(Params { density: 1.5, ..Params::default() }.validate().is_err());
        assert!(Params { cell: 1.0, ..Params::default() }.validate().is_err());
        assert!(Params { perforation: [0.5, 1.2], ..Params::default() }.validate().is_err());
        assert!(Params { vault_slot: 1.5, ..Params::default() }.validate().is_err());
        assert!(Params { bandwidth: -0.1, ..Params::default() }.validate().is_err());
        assert!(Params { load_boost: -1.0, ..Params::default() }.validate().is_err());
    }
}
