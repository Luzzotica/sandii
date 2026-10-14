//! Player-facing “wow recipes” — short reaction loops to try without reading the README.
//! Used by desktop (`sandbox_window`) and web (`sandii_web`) UIs.

use crate::world::{material, MaterialId};

/// One discoverable reaction: materials to paint, and copy that fits the north-star “whoa” moment.
#[derive(Clone, Copy, Debug)]
pub struct WowRecipe {
    /// Stable id for UI / analytics (not shown).
    pub id: &'static str,
    /// Short chip title (keep short for web buttons and desktop 5×7 font).
    pub title: &'static str,
    /// Desktop-only all-caps line (A–Z, space, `-` only — matches sandbox bitmap font).
    pub desktop_line: &'static str,
    /// What to do (web detail / desktop tip strip).
    pub how: &'static str,
    /// What you should notice.
    pub result: &'static str,
    /// Paint in order; UIs cycle selection on repeated clicks.
    pub materials: &'static [MaterialId],
}

/// At least three signature reactions from GOAL.md (plus one bonus).
pub const WOW_RECIPES: &[WowRecipe] = &[
    WowRecipe {
        id: "lava_water",
        title: "Lava + water",
        desktop_line: "LAVA MEETS WATER",
        how: "Paint a pool of water, then drop lava into or next to it.",
        result: "Steam and cooled rock (obsidian) — classic sandbox fireworks.",
        materials: &[material::LAVA, material::LIQUID],
    },
    WowRecipe {
        id: "fire_oil",
        title: "Fire chain",
        desktop_line: "FIRE ON OIL",
        how: "Paint oil, then fire (or embers) on top. Try wood or plant for a slower burn.",
        result: "Fire spreads through fuel into smoke — a satisfying chain reaction.",
        materials: &[material::FIRE, material::OIL, material::WOOD],
    },
    WowRecipe {
        id: "acid_wood",
        title: "Acid bite",
        desktop_line: "ACID ON WOOD",
        how: "Paint wood or dirt, then acid on or against it.",
        result: "Corrosion eats vulnerable solids — messy and fun.",
        materials: &[material::ACID, material::WOOD, material::DIRT],
    },
    WowRecipe {
        id: "c4_fire",
        title: "C4 boom",
        desktop_line: "C4 THEN FIRE",
        how: "Paint a C4 block, then touch it with fire or lava (or heat it hard).",
        result: "Detonation — clear space and watch the blast.",
        materials: &[material::C4, material::FIRE, material::LAVA],
    },
    WowRecipe {
        id: "metal_water_charge",
        title: "Metal → water",
        desktop_line: "METAL WATER CHARGE",
        how: "Paint metal, a water bridge, more metal, then electricity in empty next to the metal.",
        result: "Jagged charge visits water/metal (glow) and pops out — media stay metal/water.",
        materials: &[
            material::METAL,
            material::LIQUID,
            material::ELECTRICITY,
        ],
    },
];

#[inline]
pub fn wow_recipe_count() -> usize {
    WOW_RECIPES.len()
}

#[inline]
pub fn wow_recipe_at(index: usize) -> Option<&'static WowRecipe> {
    WOW_RECIPES.get(index)
}

/// Advance selection within a recipe (repeated clicks cycle materials).
pub fn recipe_material_at(recipe_index: usize, step: usize) -> Option<MaterialId> {
    let r = wow_recipe_at(recipe_index)?;
    if r.materials.is_empty() {
        return None;
    }
    Some(r.materials[step % r.materials.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_least_three_wow_recipes() {
        assert!(WOW_RECIPES.len() >= 3);
        for (i, r) in WOW_RECIPES.iter().enumerate() {
            assert!(!r.materials.is_empty(), "recipe {i} needs materials");
            assert!(recipe_material_at(i, 0).is_some());
            assert_eq!(
                recipe_material_at(i, r.materials.len()),
                recipe_material_at(i, 0)
            );
        }
    }
}
