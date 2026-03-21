use crate::world::{material, MaterialId, MaterialProps, MaterialRule, Phase};

/// One-stop definition for a built-in material. To add a new material:
/// 1. Add an ID constant in `world::material`
/// 2. Add an entry to `BUILTINS` below -- that's it for the core crate.
/// 3. Optionally add a keyboard shortcut in the sandbox example.
#[derive(Debug, Clone, Copy)]
pub struct MaterialDef {
    pub id: MaterialId,
    pub name: &'static str,
    pub props: MaterialProps,
    pub rule: MaterialRule,
    pub color_argb: u32,
}

pub const BUILTINS: &[MaterialDef] = &[
    MaterialDef {
        id: material::EMPTY,
        name: "Empty",
        props: MaterialProps {
            density: i16::MIN,
            phase: Phase::Gas,
            viscosity: 0,
            inert: false,
            max_speed: 0,
            acceleration: 0,
        },
        rule: MaterialRule { lateral_spread: 1, miscible: true },
        color_argb: 0xFF000000,
    },
    MaterialDef {
        id: material::SAND,
        name: "Sand",
        props: MaterialProps {
            density: 180,
            phase: Phase::Solid,
            viscosity: 200,
            inert: false,
            max_speed: 6,
            acceleration: 2,
        },
        rule: MaterialRule { lateral_spread: 1, miscible: false },
        color_argb: 0xFFFFC369,
    },
    MaterialDef {
        id: material::LIQUID,
        name: "Water",
        props: MaterialProps {
            density: 100,
            phase: Phase::Liquid,
            viscosity: 25,
            inert: false,
            max_speed: 4,
            acceleration: 1,
        },
        rule: MaterialRule { lateral_spread: 4, miscible: true },
        color_argb: 0xFF3264D2,
    },
    MaterialDef {
        id: material::GAS,
        name: "Gas",
        props: MaterialProps {
            density: 10,
            phase: Phase::Gas,
            viscosity: 1,
            inert: false,
            max_speed: 3,
            acceleration: 1,
        },
        rule: MaterialRule { lateral_spread: 4, miscible: true },
        color_argb: 0xFFBBBBBB,
    },
    MaterialDef {
        id: material::STATIC,
        name: "Static",
        props: MaterialProps {
            density: i16::MAX,
            phase: Phase::Solid,
            viscosity: 255,
            inert: true,
            max_speed: 0,
            acceleration: 0,
        },
        rule: MaterialRule { lateral_spread: 0, miscible: false },
        color_argb: 0xFF707070,
    },
    MaterialDef {
        id: material::RIGID,
        name: "Rigid",
        props: MaterialProps {
            density: 220,
            phase: Phase::Solid,
            viscosity: 255,
            inert: false,
            max_speed: 0,
            acceleration: 0,
        },
        rule: MaterialRule { lateral_spread: 1, miscible: false },
        color_argb: 0xFFBE6E46,
    },
    MaterialDef {
        id: material::LIGHT_LIQUID,
        name: "Light Liquid",
        props: MaterialProps {
            density: 80,
            phase: Phase::Liquid,
            viscosity: 20,
            inert: false,
            max_speed: 4,
            acceleration: 1,
        },
        rule: MaterialRule { lateral_spread: 5, miscible: true },
        color_argb: 0xFF5A96F0,
    },
    MaterialDef {
        id: material::HEAVY_LIQUID,
        name: "Heavy Liquid",
        props: MaterialProps {
            density: 140,
            phase: Phase::Liquid,
            viscosity: 35,
            inert: false,
            max_speed: 3,
            acceleration: 1,
        },
        rule: MaterialRule { lateral_spread: 3, miscible: true },
        color_argb: 0xFF2346AA,
    },
];

pub fn builtin_props() -> [MaterialProps; material::MAX_MATERIALS] {
    let mut props = [MaterialProps::default(); material::MAX_MATERIALS];
    for def in BUILTINS {
        props[def.id as usize] = def.props;
    }
    props
}

pub fn builtin_rules() -> [MaterialRule; material::MAX_MATERIALS] {
    let mut rules = [MaterialRule::default(); material::MAX_MATERIALS];
    for def in BUILTINS {
        rules[def.id as usize] = def.rule;
    }
    rules
}

pub fn builtin_palette_argb() -> [u32; material::MAX_MATERIALS] {
    let mut palette = [0xFF202020u32; material::MAX_MATERIALS];
    for def in BUILTINS {
        palette[def.id as usize] = def.color_argb;
    }
    palette
}

pub fn builtin_palette_rgba() -> [[u8; 4]; material::MAX_MATERIALS] {
    let mut palette = [[255u8, 0, 255, 255]; material::MAX_MATERIALS];
    for def in BUILTINS {
        let a = def.color_argb;
        palette[def.id as usize] = [
            ((a >> 16) & 0xFF) as u8,
            ((a >> 8) & 0xFF) as u8,
            (a & 0xFF) as u8,
            ((a >> 24) & 0xFF) as u8,
        ];
    }
    palette
}
