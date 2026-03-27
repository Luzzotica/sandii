//! Short player-facing blurbs for the web UI (not full simulation docs).

use falling_everything_core::world::material;

pub const MODE_NO_MATERIAL_PICKER: &str =
    "Material list is hidden in Explosion, Heat, and Cool. Switch to Draw or Rigid to paint materials.";
use falling_everything_core::world::MaterialId;

pub fn material_description(id: MaterialId) -> &'static str {
    match id {
        material::EMPTY => "Eraser: paints empty cells. Right-drag also erases in Draw mode.",
        material::SAND => "Granular solid: falls, piles, and spreads. Good default terrain.",
        material::LIQUID => "Water: flows, freezes to ice, boils to steam. Extinguishes fire.",
        material::GAS => "Light gas: rises and disperses.",
        material::STONE => "Fixed solid: does not move or fall.",
        material::OIL => "Liquid fuel: floats on water, burns.",
        material::FIRE => "Hot gas: spreads, heats neighbors, dies into smoke.",
        material::WAX => "Solid wax: melts to melted wax when hot enough.",
        material::MELTED_WAX => "Liquid wax: freezes back to solid wax when cool.",
        material::C4 => "Explosive inert solid: detonates from adjacent fire or when hot enough (e.g. touching lava).",
        material::SMOKE => "Dark gas: rises and fades.",
        material::PLANT => "Burnable organic: can smolder and catch fire.",
        material::ACID => "Corrosive liquid: damages vulnerable solids it touches.",
        material::EMBER => "Glowing hot particles: can ignite neighbors.",
        material::WOOD => "Solid fuel: burns slowly, good for structures.",
        material::STEAM => "Hot gas: cools and condenses toward water.",
        material::LAVA => "Molten rock: flows, freezes to obsidian when cold enough.",
        material::TORCH => "Inert prop: spawns light/fire-like behavior per rules.",
        material::WELL => "Inert prop: spawns fluid per rules.",
        material::DIRT => "Granular: similar to sand, different tuning.",
        material::GRASS => "Organic solid: can burn.",
        material::OBSIDIAN => "Frozen lava: tough solid.",
        material::ICE => "Cold solid: melts to water when warmed.",
        _ => "No description for this id.",
    }
}
