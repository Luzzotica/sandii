//! Player-facing material names/blurbs shared by desktop and web UIs.

use crate::world::{material, MaterialId};

/// Short chip label (≤6 chars, A–Z / digits / space) for dense desktop swatches.
pub fn material_short_label(id: MaterialId) -> &'static str {
    match id {
        material::EMPTY => "ERASE",
        material::SAND => "SAND",
        material::LIQUID => "WATER",
        material::GAS => "GAS",
        material::STONE => "STONE",
        material::OIL => "OIL",
        material::FIRE => "FIRE",
        material::WAX => "WAX",
        material::MELTED_WAX => "M WAX",
        material::C4 => "BOMB",
        material::SMOKE => "SMOKE",
        material::PLANT => "PLANT",
        material::ACID => "ACID",
        material::EMBER => "EMBER",
        material::WOOD => "WOOD",
        material::STEAM => "STEAM",
        material::LAVA => "LAVA",
        material::TORCH => "TORCH",
        material::WELL => "WELL",
        material::DIRT => "DIRT",
        material::GRASS => "GRASS",
        material::OBSIDIAN => "OBSID",
        material::ICE => "ICE",
        material::METAL => "METAL",
        material::ELECTRICITY => "SPARK",
        material::RIGID => "RIGID",
        _ => "???",
    }
}

/// One-line play blurb for selected material (web detail + desktop tip strip).
pub fn material_blurb(id: MaterialId) -> &'static str {
    match id {
        material::EMPTY => "Eraser: paints empty cells. Right-drag also erases in Draw mode.",
        material::SAND => "Granular solid: falls, piles, and spreads. Good default terrain.",
        material::LIQUID => "Water: flows, freezes to ice, boils to steam. Extinguishes fire.",
        material::GAS => "Light gas: rises and disperses.",
        material::STONE => "Fixed solid: does not move or fall. Static terrain.",
        material::OIL => "Liquid fuel: floats on water, burns in a chain.",
        material::FIRE => "Hot gas: spreads, heats neighbors, dies into smoke.",
        material::WAX => "Solid wax: melts when hot enough.",
        material::MELTED_WAX => "Liquid wax: freezes back to solid wax when cool.",
        material::C4 => "Explosive solid: detonates from fire, heat, or lava touch.",
        material::SMOKE => "Dark gas: rises and fades.",
        material::PLANT => "Burnable organic: can smolder and catch fire.",
        material::ACID => "Corrosive liquid: eats vulnerable solids it touches.",
        material::EMBER => "Glowing hot particles: can ignite neighbors.",
        material::WOOD => "Solid fuel: burns slowly — good for structures.",
        material::STEAM => "Hot gas: cools and condenses toward water.",
        material::LAVA => "Molten rock: flows; freezes to obsidian when cold.",
        material::TORCH => "Inert prop: keeps spawning fire-like sparks.",
        material::WELL => "Inert prop: keeps spawning fluid.",
        material::DIRT => "Granular like sand with different pile feel.",
        material::GRASS => "Organic solid: can burn.",
        material::OBSIDIAN => "Frozen lava: tough solid terrain.",
        material::ICE => "Cold solid: melts to water when warmed.",
        material::METAL => "Conductive solid: charge zigzags through without destroying it.",
        material::ELECTRICITY => {
            "Jagged spark: visits metal/water (glow), pops out. Does not replace the medium."
        }
        material::RIGID => "Engine rigid-body placeholder (not a paint brush).",
        _ => "No description for this material.",
    }
}

/// Uppercase-friendly lines for the desktop 5×7 font (A–Z, space, hyphen only).
/// Returns up to two lines of short play copy for the selected material.
pub fn material_blurb_desktop(id: MaterialId) -> [&'static str; 2] {
    match id {
        material::EMPTY => ["ERASER - PAINTS EMPTY", "RIGHT DRAG ALSO ERASES"],
        material::SAND => ["FALLS AND PILES", "GOOD DEFAULT TERRAIN"],
        material::LIQUID => ["WATER - FLOWS AND FREEZES", "EXTINGUISHES FIRE"],
        material::GAS => ["LIGHT GAS - RISES", "AND DISPERSES"],
        material::STONE => ["FIXED SOLID TERRAIN", "DOES NOT FALL"],
        material::OIL => ["FUEL - FLOATS ON WATER", "BURNS IN A CHAIN"],
        material::FIRE => ["HOT GAS - SPREADS HEAT", "DIES INTO SMOKE"],
        material::WAX => ["SOLID - MELTS WHEN HOT", "TRY WITH FIRE OR LAVA"],
        material::MELTED_WAX => ["LIQUID WAX", "FREEZES WHEN COOL"],
        material::C4 => ["EXPLOSIVE SOLID", "TOUCH WITH FIRE OR LAVA"],
        material::SMOKE => ["DARK GAS - RISES", "AND FADES"],
        material::PLANT => ["BURNABLE ORGANIC", "CAN SMOLDER"],
        material::ACID => ["CORROSIVE LIQUID", "EATS WEAK SOLIDS"],
        material::EMBER => ["GLOWING HOT BITS", "CAN IGNITE NEIGHBORS"],
        material::WOOD => ["SOLID FUEL - SLOW BURN", "GOOD FOR STRUCTURES"],
        material::STEAM => ["HOT GAS - CONDENSES", "BACK TOWARD WATER"],
        material::LAVA => ["MOLTEN ROCK - FLOWS", "FREEZES TO OBSIDIAN"],
        material::TORCH => ["PROP - SPAWNS FIRE", "PLACE AND WATCH"],
        material::WELL => ["PROP - SPAWNS FLUID", "PLACE AND WATCH"],
        material::DIRT => ["GRANULAR LIKE SAND", "DIFFERENT PILE FEEL"],
        material::GRASS => ["ORGANIC SOLID", "CAN BURN"],
        material::OBSIDIAN => ["FROZEN LAVA", "TOUGH SOLID"],
        material::ICE => ["COLD SOLID", "MELTS TO WATER"],
        material::METAL => ["CONDUCTIVE - CHARGE", "ZIGZAGS WITHOUT HARM"],
        material::ELECTRICITY => ["JAGGED SPARK VISITOR", "THROUGH METAL AND WATER"],
        _ => ["NO DESCRIPTION", ""],
    }
}

/// Display name for UI (from [`MaterialDef`], fallback to short label).
pub fn material_display_name(id: MaterialId) -> &'static str {
    if let Some(def) = super::BUILTINS.iter().find(|d| d.id == id) {
        def.name
    } else {
        material_short_label(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::{paintable_builtin_at, paintable_builtin_count};
    use crate::world::material;

    #[test]
    fn every_paintable_has_blurb_and_short_label() {
        for i in 0..paintable_builtin_count() {
            let def = paintable_builtin_at(i).expect("paintable");
            let blurb = material_blurb(def.id);
            assert!(
                !blurb.is_empty() && !blurb.contains("No description"),
                "missing blurb for {} ({})",
                def.name,
                def.id
            );
            let short = material_short_label(def.id);
            assert!(!short.is_empty());
            assert!(
                short.len() <= 6,
                "short label too long for {}: {:?}",
                def.name,
                short
            );
            let desk = material_blurb_desktop(def.id);
            assert!(!desk[0].is_empty());
            // Desktop font is A–Z / space / hyphen only.
            for line in desk {
                for b in line.bytes() {
                    assert!(
                        b.is_ascii_uppercase() || b == b' ' || b == b'-' || b == 0,
                        "desktop blurb non-font char {:?} in {:?}",
                        b as char,
                        line
                    );
                }
            }
        }
    }

    #[test]
    fn water_and_spark_are_findable() {
        assert_eq!(material_short_label(material::LIQUID), "WATER");
        assert_eq!(material_short_label(material::ELECTRICITY), "SPARK");
        assert!(material_blurb(material::METAL).to_ascii_lowercase().contains("conduct"));
    }
}
