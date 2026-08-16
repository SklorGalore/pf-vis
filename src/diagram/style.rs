//! Drawing conventions for the one-line: colours, weights and symbol proportions.
//!
//! Every dimension is in world units, a grid cell being [`GRID`]. Symbols are sized as
//! fractions of the grid so the drawing stays proportioned at any zoom.

use egui::Color32;

/// Spacing of the placement grid, in world units. All symbol sizes derive from it.
pub const GRID: f32 = 24.0;

/// How far a connection leaves a bus bar before it is allowed to turn.
pub const STUB: f32 = GRID * 0.9;

pub const BUS_WIDTH: f32 = 5.0;
pub const LINE_WIDTH: f32 = 1.6;
pub const SYMBOL_WIDTH: f32 = 1.6;

/// Radius of a transformer winding circle.
pub const WINDING_RADIUS: f32 = GRID * 0.36;
/// Distance between the two winding circle centres, less than a diameter so they interlock.
pub const WINDING_OFFSET: f32 = WINDING_RADIUS * 0.62;

pub const GEN_RADIUS: f32 = GRID * 0.44;
pub const GEN_LEAD: f32 = GRID * 1.1;
pub const LOAD_LEAD: f32 = GRID * 0.85;
pub const LOAD_SIZE: f32 = GRID * 0.5;
pub const SHUNT_LEAD: f32 = GRID * 0.7;
pub const SHUNT_SIZE: f32 = GRID * 0.42;

pub const CANVAS: Color32 = Color32::from_rgb(0xFC, 0xFC, 0xFA);
pub const GRID_DOT: Color32 = Color32::from_rgb(0xD8, 0xD8, 0xD2);
pub const INK: Color32 = Color32::from_rgb(0x20, 0x20, 0x24);
pub const LABEL: Color32 = Color32::from_rgb(0x44, 0x44, 0x4A);
pub const OUT_OF_SERVICE: Color32 = Color32::from_rgb(0x9A, 0x9A, 0xA0);
pub const SELECTED: Color32 = Color32::from_rgb(0xE8, 0x8A, 0x00);
pub const HOVERED: Color32 = Color32::from_rgb(0x3A, 0x86, 0xFF);

/// Nominal voltages get fixed colours so the same kV reads the same way across drawings.
///
/// There is no universal utility standard, so this is a conventional set rather than a
/// normative one; edit the table to match a particular utility's practice. Voltages not
/// listed take the colour of the nearest listed level.
const VOLTAGE_COLORS: &[(f64, Color32)] = &[
    (765.0, Color32::from_rgb(0x8B, 0x2F, 0x0C)),
    (500.0, Color32::from_rgb(0xB3, 0x1B, 0x1B)),
    (345.0, Color32::from_rgb(0x1C, 0x4E, 0xA8)),
    (230.0, Color32::from_rgb(0x1B, 0x7A, 0x3C)),
    (161.0, Color32::from_rgb(0x6A, 0x33, 0xA8)),
    (138.0, Color32::from_rgb(0xC0, 0x1C, 0x55)),
    (115.0, Color32::from_rgb(0xA8, 0x62, 0x00)),
    (69.0, Color32::from_rgb(0x0E, 0x7C, 0x86)),
    (46.0, Color32::from_rgb(0x5B, 0x54, 0x8C)),
    (34.5, Color32::from_rgb(0x6E, 0x74, 0x1E)),
    (25.0, Color32::from_rgb(0x9C, 0x3D, 0x8C)),
    (18.0, Color32::from_rgb(0xA6, 0x2A, 0x8E)),
    (13.8, Color32::from_rgb(0x4A, 0x4A, 0x50)),
    (4.16, Color32::from_rgb(0x77, 0x77, 0x7D)),
];

/// The colour for a nominal voltage, matched to the nearest level in the table.
///
/// Matching is on ratio rather than difference, so 13.8 kV is nearer to 18 kV than 138 kV is.
pub fn voltage_color(base_kv: f64) -> Color32 {
    if base_kv <= 0.0 {
        return INK;
    }
    VOLTAGE_COLORS
        .iter()
        .min_by(|(a, _), (b, _)| {
            let da = (a / base_kv).ln().abs();
            let db = (b / base_kv).ln().abs();
            da.total_cmp(&db)
        })
        .map(|(_, c)| *c)
        .unwrap_or(INK)
}

/// Format a nominal voltage the way it is written on a drawing: `345 kV`, `13.8 kV`.
pub fn format_kv(base_kv: f64) -> String {
    if (base_kv.fract()).abs() < 0.05 {
        format!("{:.0} kV", base_kv)
    } else {
        format!("{:.1} kV", base_kv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_levels_present_in_the_sample_case_distinctly() {
        let colors: Vec<Color32> = [345.0, 138.0, 69.0, 18.0]
            .iter()
            .map(|kv| voltage_color(*kv))
            .collect();
        for (i, a) in colors.iter().enumerate() {
            for b in &colors[i + 1..] {
                assert_ne!(a, b, "voltage levels must not share a colour");
            }
        }
    }

    #[test]
    fn unlisted_voltages_take_the_nearest_level_by_ratio() {
        assert_eq!(voltage_color(230.0), voltage_color(240.0));
        assert_eq!(voltage_color(13.8), voltage_color(14.0));
        assert_ne!(voltage_color(13.8), voltage_color(138.0));
    }

    #[test]
    fn writes_voltages_the_way_a_drawing_does() {
        assert_eq!(format_kv(345.0), "345 kV");
        assert_eq!(format_kv(13.8), "13.8 kV");
    }
}
