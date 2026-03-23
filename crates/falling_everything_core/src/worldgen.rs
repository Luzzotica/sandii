use crate::world::{material, Cell, MaterialId, Vec2i};

/// Describes one column of terrain for painting into the simulation.
pub struct TerrainColumn {
    pub x: i32,
    pub cells: Vec<(i32, MaterialId)>, // (y, material)
}

/// Configuration for procedural terrain generation.
pub struct TerrainConfig {
    pub seed: u64,
    /// World-space x range (inclusive).
    pub x_min: i32,
    pub x_max: i32,
    /// Base surface y (larger y = further down). Surface oscillates around this.
    pub surface_y: i32,
    /// Maximum amplitude of surface hills.
    pub hill_amplitude: i32,
    /// How many cells of dirt sit between grass and stone.
    pub dirt_depth: i32,
    /// Maximum depth below the surface that terrain extends.
    pub terrain_depth: i32,
    /// Probability threshold for cave carving (0.0 = no caves, 1.0 = all cave).
    pub cave_density: f64,
    /// Vertical scale of caves (smaller = more stretched horizontally).
    pub cave_scale_y: f64,
}

impl Default for TerrainConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            x_min: -1200,
            x_max: 1200,
            surface_y: 120,
            hill_amplitude: 60,
            dirt_depth: 12,
            terrain_depth: 900,
            cave_density: 0.38,
            cave_scale_y: 0.6,
        }
    }
}

/// Generate terrain columns for the given config. Each column contains
/// (y, material) pairs to paint into the world.
pub fn generate_terrain(config: &TerrainConfig) -> Vec<TerrainColumn> {
    let mut columns = Vec::with_capacity((config.x_max - config.x_min + 1) as usize);

    for x in config.x_min..=config.x_max {
        let surface = surface_height(x, config);
        let max_y = surface + config.terrain_depth;
        let dirt_bottom = surface + config.dirt_depth;

        let mut cells = Vec::new();

        for y in surface..=max_y {
            let cave = cave_noise(x, y, config);
            if cave {
                continue;
            }
            let mat = if y == surface {
                material::GRASS
            } else if y < dirt_bottom {
                material::DIRT
            } else {
                material::STATIC
            };
            cells.push((y, mat));
        }

        columns.push(TerrainColumn { x, cells });
    }

    columns
}

/// Paint generated terrain into a simulation.
pub fn paint_terrain(sim: &mut crate::Simulation, config: &TerrainConfig) {
    let columns = generate_terrain(config);
    for col in &columns {
        for &(y, mat) in &col.cells {
            sim.paint_cell(
                Vec2i::new(col.x, y),
                Cell {
                    material: mat,
                    flags: 0,
                    velocity: 0,
                    lifetime: 0,
                    variant: 0,
                    scorch: 0,
                },
            );
        }
    }
}

fn surface_height(x: i32, config: &TerrainConfig) -> i32 {
    let xf = x as f64;
    let s = config.seed as f64;
    let h1 = value_noise_1d(xf * 0.008 + s * 0.1) * config.hill_amplitude as f64;
    let h2 = value_noise_1d(xf * 0.025 + s * 0.7) * (config.hill_amplitude as f64 * 0.3);
    let h3 = value_noise_1d(xf * 0.06 + s * 1.3) * (config.hill_amplitude as f64 * 0.1);
    config.surface_y - (h1 + h2 + h3) as i32
}

fn cave_noise(x: i32, y: i32, config: &TerrainConfig) -> bool {
    let xf = x as f64 * 0.04;
    let yf = y as f64 * 0.04 * config.cave_scale_y;
    let s = config.seed as f64 * 0.31;
    let v1 = value_noise_2d(xf + s, yf + s * 0.7);
    let v2 = value_noise_2d(xf * 2.1 + s * 1.3, yf * 2.1 + s * 0.3) * 0.3;
    (v1 + v2).abs() < config.cave_density * 0.5
}

// --- Minimal value noise (no external deps) ---

fn hash_f64(x: f64) -> f64 {
    let n = (x * 127.1 + 311.7).sin() * 43758.5453;
    n - n.floor()
}

fn hash_2d(x: f64, y: f64) -> f64 {
    let n = (x * 127.1 + y * 311.7).sin() * 43758.5453;
    n - n.floor()
}

fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn value_noise_1d(x: f64) -> f64 {
    let xi = x.floor();
    let xf = x - xi;
    let a = hash_f64(xi);
    let b = hash_f64(xi + 1.0);
    let t = smoothstep(xf);
    (a + (b - a) * t) * 2.0 - 1.0 // range [-1, 1]
}

fn value_noise_2d(x: f64, y: f64) -> f64 {
    let xi = x.floor();
    let yi = y.floor();
    let xf = x - xi;
    let yf = y - yi;
    let aa = hash_2d(xi, yi);
    let ba = hash_2d(xi + 1.0, yi);
    let ab = hash_2d(xi, yi + 1.0);
    let bb = hash_2d(xi + 1.0, yi + 1.0);
    let tx = smoothstep(xf);
    let ty = smoothstep(yf);
    let top = aa + (ba - aa) * tx;
    let bot = ab + (bb - ab) * tx;
    (top + (bot - top) * ty) * 2.0 - 1.0 // range [-1, 1]
}
