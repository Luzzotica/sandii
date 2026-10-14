//! Web UI blurbs — re-export shared core copy so desktop and web stay in parity.

pub use falling_everything_core::{
    material_blurb as material_description, material_short_label,
};

pub const MODE_NO_MATERIAL_PICKER: &str =
    "Material list is hidden in Explosion, Heat, and Cool. Switch to Draw or Rigid to paint materials.";
