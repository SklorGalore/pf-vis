//! Painting the one-line.
//!
//! Symbols follow the usual single-line conventions: heavy bus bars coloured by nominal
//! voltage, thin orthogonal runs, interlocking circles for transformer windings, a circle with
//! a sine for a machine, an arrow for a load, plates for a capacitor and a coil for a reactor,
//! both grounded. Out-of-service equipment is drawn dashed and grey.

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2};

use super::layout::{ElemGeom, EquipGeom, EquipKind, Geometry, Symbol};
use super::style::*;
use super::{Camera, Orientation};
use crate::model::{ElemId, Network};

/// Something the pointer is over or the user has selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Bus(i32),
    Element(ElemId),
}

pub struct Options {
    pub show_grid: bool,
    pub show_labels: bool,
    pub selected: Option<Pick>,
    pub hovered: Option<Pick>,
}

/// Maps world coordinates onto the screen and keeps line weights sane at any zoom.
struct Pen<'a> {
    painter: &'a Painter,
    camera: Camera,
    viewport: Rect,
}

impl Pen<'_> {
    fn at(&self, world: Pos2) -> Pos2 {
        self.camera.to_screen(world, self.viewport)
    }

    fn len(&self, world: f32) -> f32 {
        world * self.camera.zoom
    }

    /// Line weights scale with zoom but never vanish or turn into slabs.
    fn stroke(&self, width: f32, color: Color32) -> Stroke {
        Stroke::new(
            (width * self.camera.zoom).clamp(width * 0.6, width * 2.5),
            color,
        )
    }

    fn path(&self, world: &[Pos2]) -> Vec<Pos2> {
        world.iter().map(|p| self.at(*p)).collect()
    }
}

pub fn draw(
    painter: &Painter,
    viewport: Rect,
    camera: Camera,
    net: &Network,
    geometry: &Geometry,
    options: &Options,
) {
    let pen = Pen {
        painter,
        camera,
        viewport,
    };
    painter.rect_filled(viewport, 0.0, CANVAS);
    if options.show_grid {
        draw_grid(&pen);
    }

    // Runs first so the bars and symbols sit on top of them.
    for element in &geometry.elements {
        draw_element(&pen, element, options);
    }
    for element in &geometry.elements {
        draw_transformer(&pen, element, options);
    }
    for equipment in &geometry.equipment {
        draw_equipment(&pen, geometry, equipment);
    }
    for (bus, geom) in &geometry.buses {
        let highlight = highlight_for(options, Pick::Bus(*bus));
        let color = voltage_color(geom.base_kv);
        if let Some(glow) = highlight {
            pen.painter.line_segment(
                [pen.at(geom.a), pen.at(geom.b)],
                pen.stroke(BUS_WIDTH + 5.0, glow.gamma_multiply(0.45)),
            );
        }
        pen.painter.line_segment(
            [pen.at(geom.a), pen.at(geom.b)],
            pen.stroke(BUS_WIDTH, color),
        );
    }

    if options.show_labels && camera.zoom > 0.35 {
        for (bus, geom) in &geometry.buses {
            draw_bus_label(&pen, net, *bus, geom);
        }
        for element in &geometry.elements {
            if let Some(ckt) = &element.ckt_label {
                let font = FontId::proportional((9.0 * camera.zoom).clamp(7.0, 16.0));
                pen.painter.text(
                    pen.at(element.label_at) + Vec2::new(4.0, -4.0),
                    Align2::LEFT_BOTTOM,
                    ckt,
                    font,
                    LABEL,
                );
            }
        }
    }
}

fn draw_grid(pen: &Pen) {
    // Coarsen the grid as it is zoomed out so the dots never merge into a wash.
    let mut step = GRID;
    while pen.len(step) < 10.0 {
        step *= 2.0;
    }
    let min = pen.camera.to_world(pen.viewport.min, pen.viewport);
    let max = pen.camera.to_world(pen.viewport.max, pen.viewport);
    let start = |v: f32| (v / step).floor() * step;

    let mut dots = Vec::new();
    let mut y = start(min.y);
    while y <= max.y {
        let mut x = start(min.x);
        while x <= max.x {
            dots.push(Shape::circle_filled(pen.at(pos2(x, y)), 1.0, GRID_DOT));
            x += step;
        }
        y += step;
    }
    pen.painter.extend(dots);
}

fn highlight_for(options: &Options, pick: Pick) -> Option<Color32> {
    if options.selected == Some(pick) {
        Some(SELECTED)
    } else if options.hovered == Some(pick) {
        Some(HOVERED)
    } else {
        None
    }
}

/// Each leg takes its own voltage's colour, unless the element is out of service or picked.
fn leg_color(element: &ElemGeom, base_kv: f64, options: &Options) -> Color32 {
    if !element.in_service {
        OUT_OF_SERVICE
    } else if let Some(glow) = highlight_for(options, Pick::Element(element.id)) {
        glow
    } else {
        voltage_color(base_kv)
    }
}

fn draw_element(pen: &Pen, element: &ElemGeom, options: &Options) {
    let width = if highlight_for(options, Pick::Element(element.id)).is_some() {
        LINE_WIDTH * 2.0
    } else {
        LINE_WIDTH
    };
    for leg in &element.legs {
        let stroke = pen.stroke(width, leg_color(element, leg.base_kv, options));
        let points = pen.path(&leg.points);
        if element.in_service {
            pen.painter.add(Shape::line(points, stroke));
        } else {
            pen.painter.extend(Shape::dashed_line(
                &points,
                stroke,
                pen.len(6.0),
                pen.len(4.0),
            ));
        }
    }
}

/// Each winding circle takes the colour of the side it belongs to, so the symbol shows the
/// change of voltage as plainly as the run into it does.
fn draw_transformer(pen: &Pen, element: &ElemGeom, options: &Options) {
    let Some(symbol) = &element.symbol else {
        return;
    };
    let colors: Vec<Color32> = element
        .legs
        .iter()
        .map(|leg| leg_color(element, leg.base_kv, options))
        .collect();
    let first = colors.first().copied().unwrap_or(INK);
    let radius = pen.len(WINDING_RADIUS);

    match symbol {
        Symbol::TwoWinding {
            at,
            dir,
            phase_shifter,
        } => {
            let offset = *dir * WINDING_OFFSET;
            let sides = [
                (pen.at(*at - offset), first),
                (
                    pen.at(*at + offset),
                    colors.last().copied().unwrap_or(first),
                ),
            ];
            // Fill first so the run is hidden behind the windings, then outline both.
            for (c, _) in sides {
                pen.painter.add(Shape::circle_filled(c, radius, CANVAS));
            }
            for (c, color) in sides {
                pen.painter.add(Shape::circle_stroke(
                    c,
                    radius,
                    pen.stroke(SYMBOL_WIDTH, color),
                ));
            }
            if *phase_shifter {
                // The standard mark for a phase shifting transformer: a slash across the symbol.
                let across = Vec2::new(-dir.y, dir.x) * WINDING_RADIUS * 1.5;
                let along = *dir * WINDING_RADIUS * 1.5;
                pen.painter.line_segment(
                    [pen.at(*at - across - along), pen.at(*at + across + along)],
                    pen.stroke(SYMBOL_WIDTH, first),
                );
            }
        }
        Symbol::ThreeWinding { star, windings } => {
            for (center, _) in windings {
                pen.painter
                    .add(Shape::circle_filled(pen.at(*center), radius, CANVAS));
            }
            for (i, (center, _)) in windings.iter().enumerate() {
                let color = colors.get(i).copied().unwrap_or(first);
                pen.painter.add(Shape::circle_stroke(
                    pen.at(*center),
                    radius,
                    pen.stroke(SYMBOL_WIDTH, color),
                ));
            }
            pen.painter
                .add(Shape::circle_filled(pen.at(*star), pen.len(1.5), INK));
        }
    }
}

fn draw_equipment(pen: &Pen, geometry: &Geometry, equipment: &EquipGeom) {
    let base_kv = geometry
        .buses
        .get(&equipment.bus)
        .map_or(0.0, |b| b.base_kv);
    let color = if equipment.in_service {
        voltage_color(base_kv)
    } else {
        OUT_OF_SERVICE
    };
    let stroke = pen.stroke(SYMBOL_WIDTH, color);
    let (root, dir) = (equipment.root, equipment.dir);
    let across = Vec2::new(-dir.y, dir.x);

    match equipment.kind {
        EquipKind::Generator => {
            let center = root + dir * (GEN_LEAD + GEN_RADIUS);
            pen.painter
                .line_segment([pen.at(root), pen.at(root + dir * GEN_LEAD)], stroke);
            pen.painter.add(Shape::circle_filled(
                pen.at(center),
                pen.len(GEN_RADIUS),
                CANVAS,
            ));
            pen.painter.add(Shape::circle_stroke(
                pen.at(center),
                pen.len(GEN_RADIUS),
                stroke,
            ));
            // The sine inside the circle is what distinguishes a machine from a bare node.
            let wave: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0 * 2.0 - 1.0;
                    let world = center
                        + across * (t * GEN_RADIUS * 0.62)
                        + dir * ((t * std::f32::consts::PI).sin() * GEN_RADIUS * 0.34);
                    pen.at(world)
                })
                .collect();
            pen.painter.add(Shape::line(wave, stroke));
        }
        EquipKind::Load => {
            let tip = root + dir * (LOAD_LEAD + LOAD_SIZE);
            let base = root + dir * LOAD_LEAD;
            pen.painter
                .line_segment([pen.at(root), pen.at(base)], stroke);
            pen.painter.add(Shape::convex_polygon(
                vec![
                    pen.at(tip),
                    pen.at(base + across * LOAD_SIZE * 0.5),
                    pen.at(base - across * LOAD_SIZE * 0.5),
                ],
                color,
                stroke,
            ));
        }
        EquipKind::Capacitor => {
            let first = root + dir * SHUNT_LEAD;
            let second = first + dir * SHUNT_SIZE * 0.38;
            pen.painter
                .line_segment([pen.at(root), pen.at(first)], stroke);
            for plate in [first, second] {
                pen.painter.line_segment(
                    [
                        pen.at(plate - across * SHUNT_SIZE * 0.6),
                        pen.at(plate + across * SHUNT_SIZE * 0.6),
                    ],
                    stroke,
                );
            }
            let ground = second + dir * SHUNT_LEAD * 0.55;
            pen.painter
                .line_segment([pen.at(second), pen.at(ground)], stroke);
            draw_ground(pen, ground, dir, stroke);
        }
        EquipKind::Reactor => {
            let start = root + dir * SHUNT_LEAD * 0.6;
            pen.painter
                .line_segment([pen.at(root), pen.at(start)], stroke);
            // Three half turns make the coil read as an inductor rather than a resistor.
            let turn = SHUNT_SIZE * 0.42;
            let mut coil = Vec::new();
            for i in 0..=36 {
                let t = i as f32 / 36.0;
                let angle = t * 3.0 * std::f32::consts::PI;
                let world = start + dir * (t * turn * 3.0) + across * (angle.sin().abs() * turn);
                coil.push(pen.at(world));
            }
            pen.painter.add(Shape::line(coil, stroke));
            let end = start + dir * turn * 3.0;
            let ground = end + dir * SHUNT_LEAD * 0.55;
            pen.painter
                .line_segment([pen.at(end), pen.at(ground)], stroke);
            draw_ground(pen, ground, dir, stroke);
        }
    }
}

/// The three shortening bars that mark a ground connection.
fn draw_ground(pen: &Pen, at: Pos2, dir: Vec2, stroke: Stroke) {
    let across = Vec2::new(-dir.y, dir.x);
    for (i, half) in [0.30, 0.19, 0.09].iter().enumerate() {
        let p = at + dir * (i as f32 * GRID * 0.09);
        pen.painter.line_segment(
            [
                pen.at(p - across * GRID * *half),
                pen.at(p + across * GRID * *half),
            ],
            stroke,
        );
    }
}

/// Bus identification, always horizontal so it stays readable however the bar runs.
fn draw_bus_label(pen: &Pen, net: &Network, bus: i32, geom: &super::layout::BusGeom) {
    let Some(record) = net.bus(bus) else { return };
    let size = (10.0 * pen.camera.zoom).clamp(8.0, 18.0);
    let (anchor, align) = match geom.orient {
        Orientation::Horizontal => (pen.at(geom.a) + Vec2::new(-5.0, 0.0), Align2::RIGHT_BOTTOM),
        Orientation::Vertical => (pen.at(geom.a) + Vec2::new(0.0, -5.0), Align2::CENTER_BOTTOM),
    };
    let name = if record.name.is_empty() {
        format!("{bus}")
    } else {
        format!("{bus} {}", record.name)
    };
    pen.painter
        .text(anchor, align, name, FontId::proportional(size), INK);
    pen.painter.text(
        anchor + Vec2::new(0.0, size * 1.15),
        align,
        format_kv(record.base_kv),
        FontId::proportional(size * 0.85),
        LABEL,
    );
}
