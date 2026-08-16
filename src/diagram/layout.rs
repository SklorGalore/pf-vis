//! Turning placed buses into drawable geometry.
//!
//! Everything here works in world coordinates. The rules are the ones a draughtsman applies by
//! hand: a bus bar is long enough to carry its connections, a connection leaves the bar on the
//! side facing where it is going, connections are ordered along the bar so they do not cross,
//! and every run is orthogonal.

use std::collections::BTreeMap;

use egui::{Pos2, Rect, Vec2, pos2};

use super::style::{GEN_LEAD, GEN_RADIUS, GRID, STUB, WINDING_RADIUS};
use super::{Diagram, Orientation};
use crate::model::{ElemId, Element, Network};

/// A bus bar: where it sits and how far it runs.
#[derive(Debug, Clone, Copy)]
pub struct BusGeom {
    pub a: Pos2,
    pub b: Pos2,
    pub orient: Orientation,
    pub base_kv: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquipKind {
    Generator,
    Load,
    Capacitor,
    Reactor,
}

/// A generator, load or shunt hanging off a bus.
#[derive(Debug, Clone, Copy)]
pub struct EquipGeom {
    pub bus: i32,
    pub kind: EquipKind,
    /// Where the lead meets the bar.
    pub root: Pos2,
    /// Direction the lead runs, away from the bar.
    pub dir: Vec2,
    pub in_service: bool,
}

/// Where a transformer's winding circles go on the run between its buses.
#[derive(Debug, Clone)]
pub enum Symbol {
    TwoWinding {
        at: Pos2,
        dir: Vec2,
        phase_shifter: bool,
    },
    ThreeWinding {
        star: Pos2,
        windings: Vec<(Pos2, Vec2)>,
    },
}

/// A drawn branch or transformer: one run per pair of terminals, three for a star connection.
#[derive(Debug, Clone)]
pub struct ElemGeom {
    pub id: ElemId,
    pub legs: Vec<Leg>,
    pub symbol: Option<Symbol>,
    pub in_service: bool,
    /// Circuit identifier, drawn only where a parallel circuit makes it necessary.
    pub ckt_label: Option<String>,
    pub label_at: Pos2,
}

#[derive(Debug, Default)]
pub struct Geometry {
    pub buses: BTreeMap<i32, BusGeom>,
    pub elements: Vec<ElemGeom>,
    pub equipment: Vec<EquipGeom>,
}

/// A stretch of a run at one nominal voltage.
///
/// A line is a single leg. A transformer is one leg per winding, meeting at the symbol, so
/// each side is drawn in its own voltage's colour.
#[derive(Debug, Clone)]
pub struct Leg {
    pub points: Vec<Pos2>,
    pub base_kv: f64,
}

/// Where a run meets a bus bar, and which way it leaves.
#[derive(Debug, Clone, Copy)]
struct Termination {
    point: Pos2,
    dir: Vec2,
}

/// One thing that needs a place on a bus bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    Element(ElemId),
    Equipment(usize),
}

pub fn build(net: &Network, diagram: &Diagram, show_equipment: bool) -> Geometry {
    let visible = diagram.visible_elements(net);
    let centers: BTreeMap<i32, Pos2> = diagram
        .placed
        .iter()
        .map(|(bus, node)| (*bus, node.center()))
        .collect();

    // Three-winding transformers meet at a star point between their buses.
    let stars: BTreeMap<ElemId, Pos2> = visible
        .iter()
        .filter_map(|id| {
            let terminals = net.element(*id).terminals();
            (terminals.len() > 2).then(|| (*id, centroid(&terminals, &centers)))
        })
        .collect();

    let equipment = collect_equipment(net, diagram, show_equipment);

    // What each bus has to carry, and which way it is headed.
    let mut requests: BTreeMap<i32, Vec<(Slot, Pos2)>> =
        diagram.placed.keys().map(|b| (*b, Vec::new())).collect();
    for id in &visible {
        let terminals = net.element(*id).terminals();
        for bus in &terminals {
            let target = match stars.get(id) {
                Some(star) => *star,
                None => {
                    let other = terminals
                        .iter()
                        .find(|t| *t != bus)
                        .copied()
                        .unwrap_or(*bus);
                    centers[&other]
                }
            };
            requests
                .entry(*bus)
                .or_default()
                .push((Slot::Element(*id), target));
        }
    }

    let mut geometry = Geometry::default();
    let mut terminations: BTreeMap<(i32, Slot), Termination> = BTreeMap::new();

    for (bus, node) in &diagram.placed {
        let center = node.center();
        let orient = node.orient;
        let along = orient.along();
        let [side_a, side_b] = orient.sides();

        // Connections leave on the side facing their destination; equipment always hangs off
        // the second side, which is the underside of a horizontal bar.
        let mut on_a: Vec<(Slot, Pos2)> = Vec::new();
        let mut on_b: Vec<(Slot, Pos2)> = Vec::new();
        let mut level: Vec<(Slot, Pos2)> = Vec::new();
        for (slot, target) in requests.get(bus).map(Vec::as_slice).unwrap_or(&[]) {
            let offset = *target - center;
            match offset.dot(side_a).total_cmp(&offset.dot(side_b)) {
                std::cmp::Ordering::Greater => on_a.push((*slot, *target)),
                std::cmp::Ordering::Less => on_b.push((*slot, *target)),
                // A destination level with the bar could go either way; decide once the
                // clearly-sided connections are in, and even the two sides up.
                std::cmp::Ordering::Equal => level.push((*slot, *target)),
            }
        }
        for entry in level {
            if on_a.len() < on_b.len() {
                on_a.push(entry);
            } else {
                on_b.push(entry);
            }
        }
        // Ordering by position along the bar is what keeps runs from crossing each other.
        on_a.sort_by(|(_, p), (_, q)| along.dot(p.to_vec2()).total_cmp(&along.dot(q.to_vec2())));
        on_b.sort_by(|(_, p), (_, q)| along.dot(p.to_vec2()).total_cmp(&along.dot(q.to_vec2())));
        for (index, eq) in equipment.iter().enumerate() {
            if eq.bus == *bus {
                on_b.push((Slot::Equipment(index), center + side_b * GRID));
            }
        }

        let span = (on_a.len().max(on_b.len()) as f32 + 1.0).clamp(2.0, 24.0);
        let length = span * GRID;
        let geom = BusGeom {
            a: center - along * length / 2.0,
            b: center + along * length / 2.0,
            orient,
            base_kv: net.bus(*bus).map_or(0.0, |b| b.base_kv),
        };
        geometry.buses.insert(*bus, geom);

        for (side, slots) in [(side_a, &on_a), (side_b, &on_b)] {
            for (i, (slot, _)) in slots.iter().enumerate() {
                let t = (i + 1) as f32 / (slots.len() + 1) as f32;
                let point = geom.a + (geom.b - geom.a) * t;
                terminations.insert((*bus, *slot), Termination { point, dir: side });
            }
        }
    }

    // Equipment leads start from the slot the bus assigned them.
    geometry.equipment = equipment
        .into_iter()
        .enumerate()
        .filter_map(|(index, mut eq)| {
            let t = terminations.get(&(eq.bus, Slot::Equipment(index)))?;
            eq.root = t.point;
            eq.dir = t.dir;
            Some(eq)
        })
        .collect();

    for id in visible {
        let element = net.element(id);
        let terminals = element.terminals();
        let kv = |bus: i32| net.bus(bus).map_or(0.0, |b| b.base_kv);

        let (legs, label_at) = match stars.get(&id) {
            Some(star) => {
                let legs: Vec<Leg> = terminals
                    .iter()
                    .filter_map(|bus| {
                        let t = terminations.get(&(*bus, Slot::Element(id)))?;
                        Some(Leg {
                            points: route_to_point(*t, *star),
                            base_kv: kv(*bus),
                        })
                    })
                    .collect();
                (legs, *star)
            }
            None => {
                let ends: Vec<Termination> = terminals
                    .iter()
                    .filter_map(|bus| terminations.get(&(*bus, Slot::Element(id))).copied())
                    .collect();
                let [a, b] = ends.as_slice() else { continue };
                let path = route(*a, *b);
                let (near, far, mid, _) = split_at_midpoint(&path);
                let (kv_a, kv_b) = (kv(terminals[0]), kv(terminals[1]));
                // Only a change of voltage justifies breaking the run into two strokes.
                let legs = if (kv_a - kv_b).abs() < 0.05 {
                    vec![Leg {
                        points: path,
                        base_kv: kv_a,
                    }]
                } else {
                    vec![
                        Leg {
                            points: near,
                            base_kv: kv_a,
                        },
                        Leg {
                            points: far,
                            base_kv: kv_b,
                        },
                    ]
                };
                (legs, mid)
            }
        };
        if legs.is_empty() {
            continue;
        }

        geometry.elements.push(ElemGeom {
            id,
            symbol: transformer_symbol(&element, &legs, stars.get(&id).copied()),
            in_service: element.in_service(),
            ckt_label: net
                .has_parallel(id)
                .then(|| element.ckt().to_string())
                .filter(|c| !c.is_empty()),
            label_at,
            legs,
        });
    }

    geometry
}

fn collect_equipment(net: &Network, diagram: &Diagram, show: bool) -> Vec<EquipGeom> {
    if !show {
        return Vec::new();
    }
    let mut out = Vec::new();
    for bus in diagram.placed.keys() {
        let Some(attached) = net.attached(*bus) else {
            continue;
        };
        let mut push = |kind: EquipKind, in_service: bool| {
            out.push(EquipGeom {
                bus: *bus,
                kind,
                root: Pos2::ZERO,
                dir: Vec2::ZERO,
                in_service,
            })
        };
        for i in &attached.generators {
            push(EquipKind::Generator, net.generators[*i].status != 0);
        }
        for i in &attached.loads {
            push(EquipKind::Load, net.loads[*i].status != 0);
        }
        for i in &attached.fixed_shunts {
            let shunt = &net.fixed_shunts[*i];
            push(shunt_kind(shunt.bl), shunt.status != 0);
        }
        for i in &attached.switched_shunts {
            let shunt = &net.switched_shunts[*i];
            push(shunt_kind(shunt.binit), shunt.status != 0);
        }
    }
    out
}

/// Positive susceptance is capacitive, negative is a reactor.
fn shunt_kind(b: f64) -> EquipKind {
    if b < 0.0 {
        EquipKind::Reactor
    } else {
        EquipKind::Capacitor
    }
}

fn centroid(buses: &[i32], centers: &BTreeMap<i32, Pos2>) -> Pos2 {
    let mut sum = Vec2::ZERO;
    let mut n = 0.0;
    for bus in buses {
        if let Some(p) = centers.get(bus) {
            sum += p.to_vec2();
            n += 1.0;
        }
    }
    if n == 0.0 {
        Pos2::ZERO
    } else {
        (sum / n).to_pos2()
    }
}

/// An orthogonal run between two bus terminations.
///
/// Both ends leave their bar straight for [`STUB`] before turning, which is what makes the
/// connection read as belonging to that bus.
fn route(a: Termination, b: Termination) -> Vec<Pos2> {
    let a1 = a.point + a.dir * STUB;
    let b1 = b.point + b.dir * STUB;
    let vertical = |d: Vec2| d.x.abs() < 0.5;

    let mut path = vec![a.point, a1];
    match (vertical(a.dir), vertical(b.dir)) {
        (true, true) => {
            let mid = (a1.y + b1.y) / 2.0;
            path.push(pos2(a1.x, mid));
            path.push(pos2(b1.x, mid));
        }
        (false, false) => {
            let mid = (a1.x + b1.x) / 2.0;
            path.push(pos2(mid, a1.y));
            path.push(pos2(mid, b1.y));
        }
        (true, false) => path.push(pos2(b1.x, a1.y)),
        (false, true) => path.push(pos2(a1.x, b1.y)),
    }
    path.push(b1);
    path.push(b.point);
    dedupe(path)
}

/// An orthogonal run from a bus termination to a free point, such as a star node.
fn route_to_point(a: Termination, target: Pos2) -> Vec<Pos2> {
    let a1 = a.point + a.dir * STUB;
    let corner = if a.dir.x.abs() < 0.5 {
        pos2(a1.x, target.y)
    } else {
        pos2(target.x, a1.y)
    };
    dedupe(vec![a.point, a1, corner, target])
}

fn dedupe(points: Vec<Pos2>) -> Vec<Pos2> {
    let mut out: Vec<Pos2> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().is_none_or(|q| (p - *q).length() > 0.01) {
            out.push(p);
        }
    }
    out
}

/// Cut a polyline in half by length: the two halves, the point they meet, and the direction
/// of travel there. The meeting point is where a transformer's windings go.
fn split_at_midpoint(path: &[Pos2]) -> (Vec<Pos2>, Vec<Pos2>, Pos2, Vec2) {
    let total: f32 = path.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    let half = total / 2.0;
    let mut travelled = 0.0;

    for (i, w) in path.windows(2).enumerate() {
        let span = (w[1] - w[0]).length();
        if travelled + span >= half && span > 0.0 {
            let t = (half - travelled) / span;
            let mid = w[0] + (w[1] - w[0]) * t;
            let mut near = path[..=i].to_vec();
            near.push(mid);
            let mut far = vec![mid];
            far.extend_from_slice(&path[i + 1..]);
            return (near, far, mid, (w[1] - w[0]) / span);
        }
        travelled += span;
    }
    let only = path.first().copied().unwrap_or(Pos2::ZERO);
    (vec![only], vec![only], only, Vec2::X)
}

fn transformer_symbol(element: &Element, legs: &[Leg], star: Option<Pos2>) -> Option<Symbol> {
    let Element::Transformer(xf) = element else {
        return None;
    };
    match star {
        Some(star) => {
            // One winding circle per leg, set back from the star so all three are visible.
            let windings = legs
                .iter()
                .filter_map(|leg| {
                    let toward_star = *leg.points.last()?;
                    let previous = *leg.points.get(leg.points.len().checked_sub(2)?)?;
                    let dir = (toward_star - previous).normalized();
                    Some((star - dir * WINDING_RADIUS * 1.7, dir))
                })
                .collect();
            Some(Symbol::ThreeWinding { star, windings })
        }
        None => {
            // The windings straddle the point where the two legs meet.
            let first = legs.first()?;
            let (_, _, at, dir) = split_at_midpoint(&first.points);
            let (at, dir) = if legs.len() > 1 {
                let end = *first.points.last()?;
                let previous = *first.points.get(first.points.len().checked_sub(2)?)?;
                (end, (end - previous).normalized())
            } else {
                (at, dir)
            };
            Some(Symbol::TwoWinding {
                at,
                dir,
                phase_shifter: xf.is_phase_shifter(),
            })
        }
    }
}

impl Geometry {
    /// Everything drawn, in world coordinates, for zoom-to-fit.
    pub fn bounds(&self) -> Option<Rect> {
        let mut rect: Option<Rect> = None;
        let mut extend = |p: Pos2| {
            rect = Some(match rect {
                Some(r) => r.union(Rect::from_min_max(p, p)),
                None => Rect::from_min_max(p, p),
            });
        };
        for bus in self.buses.values() {
            // Identification text sits off the far end of the bar, so leave room for it or
            // fitting the view will cut every label off.
            extend(bus.a - bus.orient.along() * GRID * 5.0);
            extend(bus.b);
        }
        for element in &self.elements {
            for leg in &element.legs {
                for p in &leg.points {
                    extend(*p);
                }
            }
        }
        for eq in &self.equipment {
            extend(eq.root + eq.dir * (GEN_LEAD + GEN_RADIUS * 2.0));
        }
        rect
    }

    /// The bus whose bar is within `tol` of `world`, nearest first.
    pub fn hit_bus(&self, world: Pos2, tol: f32) -> Option<i32> {
        self.buses
            .iter()
            .map(|(bus, geom)| (*bus, distance_to_segment(world, geom.a, geom.b)))
            .filter(|(_, d)| *d <= tol)
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(bus, _)| bus)
    }

    /// The element whose run is within `tol` of `world`, nearest first.
    pub fn hit_element(&self, world: Pos2, tol: f32) -> Option<ElemId> {
        self.elements
            .iter()
            .map(|element| {
                let d = element
                    .legs
                    .iter()
                    .flat_map(|leg| leg.points.windows(2))
                    .map(|w| distance_to_segment(world, w[0], w[1]))
                    .fold(f32::INFINITY, f32::min);
                (element.id, d)
            })
            .filter(|(_, d)| *d <= tol)
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(id, _)| id)
    }
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_sq();
    if len_sq < 1e-6 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// Somewhere to put a bus that is being brought onto the drawing next to `from`.
///
/// Higher voltages go above their neighbour and lower voltages below, which is how a one-line
/// is normally arranged; equal voltages go alongside. From there the search spirals outward
/// until it finds a cell clear of every bus already placed.
pub fn find_free_slot(net: &Network, diagram: &Diagram, from: i32, new_bus: i32) -> (i32, i32) {
    let parent = diagram
        .placed
        .get(&from)
        .copied()
        .unwrap_or(super::BusNode {
            gx: 0,
            gy: 0,
            orient: Orientation::Horizontal,
        });
    let kv = |bus: i32| net.bus(bus).map_or(0.0, |b| b.base_kv);
    let (parent_kv, new_kv) = (kv(from), kv(new_bus));

    let preferred = if new_kv > parent_kv * 1.05 {
        (0, -5)
    } else if new_kv < parent_kv * 0.95 {
        (0, 5)
    } else {
        (8, 0)
    };
    free_cell_near(diagram, (parent.gx + preferred.0, parent.gy + preferred.1))
}

/// The nearest grid cell to `anchor` that no placed bus is already crowding.
pub fn free_cell_near(diagram: &Diagram, anchor: (i32, i32)) -> (i32, i32) {
    // Bars run horizontally, so clearance is wider across than down.
    let free = |gx: i32, gy: i32| {
        diagram
            .placed
            .values()
            .all(|n| (n.gx - gx).abs() >= 7 || (n.gy - gy).abs() >= 3)
    };

    for radius in 0..=32i32 {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs().max(dy.abs()) != radius {
                    continue;
                }
                let (gx, gy) = (anchor.0 + dx, anchor.1 + dy);
                if free(gx, gy) {
                    return (gx, gy);
                }
            }
        }
    }
    anchor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::Diagram;
    use crate::psse;
    use std::path::Path;

    fn sample_net() -> Network {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("cases/MemphisCase2026_Mar7.RAW");
        Network::from_raw(psse::parse_file(&path).expect("sample case parses"))
    }

    #[test]
    fn a_run_leaves_both_bars_before_it_turns() {
        let a = Termination {
            point: pos2(0.0, 0.0),
            dir: Vec2::new(0.0, 1.0),
        };
        let b = Termination {
            point: pos2(100.0, 200.0),
            dir: Vec2::new(0.0, -1.0),
        };
        let path = route(a, b);
        assert_eq!(path.first(), Some(&pos2(0.0, 0.0)));
        assert_eq!(path.last(), Some(&pos2(100.0, 200.0)));
        assert_eq!(path[1], pos2(0.0, STUB));
        // Every leg is horizontal or vertical.
        for w in path.windows(2) {
            let d = w[1] - w[0];
            assert!(
                d.x.abs() < 0.01 || d.y.abs() < 0.01,
                "{d:?} is not orthogonal"
            );
        }
    }

    #[test]
    fn a_bus_bar_grows_with_the_number_of_connections() {
        let net = sample_net();
        let mut sparse = Diagram::default();
        sparse.place(2, 0, 0);
        sparse.place(3, 0, 6);
        let short = build(&net, &sparse, false).buses[&2];

        let mut busy = sparse.clone();
        for (id, _) in busy.growth_options(&net, 2).clone() {
            busy.grow(&net, 2, id);
        }
        let long = build(&net, &busy, false).buses[&2];
        assert!(
            (long.b - long.a).length() > (short.b - short.a).length(),
            "a busier bus should have a longer bar"
        );
    }

    #[test]
    fn a_transformer_gets_its_winding_symbol_and_a_line_does_not() {
        let net = sample_net();
        let mut diagram = Diagram::default();
        diagram.place(2, 0, 0);
        diagram.place(3, 0, 6);
        diagram.place(722, 12, 0);
        let geometry = build(&net, &diagram, false);
        for element in &geometry.elements {
            let is_xfmr = matches!(element.id, ElemId::Transformer(_));
            assert_eq!(
                element.symbol.is_some(),
                is_xfmr,
                "{}",
                net.label(element.id)
            );
        }
    }

    #[test]
    fn equipment_hangs_off_the_underside_of_the_bar() {
        let net = sample_net();
        let mut diagram = Diagram::default();
        diagram.place(808, 0, 0);
        let geometry = build(&net, &diagram, true);
        assert_eq!(geometry.equipment.len(), 1);
        let generator = geometry.equipment[0];
        assert_eq!(generator.kind, EquipKind::Generator);
        assert!(generator.dir.y > 0.0, "leads run downward from the bar");
    }

    #[test]
    fn a_new_bus_lands_clear_of_the_one_it_grew_from() {
        let net = sample_net();
        let mut diagram = Diagram::default();
        diagram.place(2, 0, 0);
        // Bus 3 is the 69 kV side of the transformer, so it belongs below the 138 kV bus.
        let (gx, gy) = find_free_slot(&net, &diagram, 2, 3);
        assert!(gy > 0, "lower voltage should sit below");
        assert!(
            gx.abs() >= 7 || gy.abs() >= 3,
            "must clear the existing bar"
        );
    }

    #[test]
    fn growing_two_levels_out_yields_a_drawable_sheet() {
        // The path the Add buttons drive: grow, then lay out everything that became visible.
        let net = sample_net();
        let mut diagram = Diagram::default();
        diagram.place(2, 0, 0);
        for _ in 0..2 {
            let frontier: Vec<i32> = diagram.placed.keys().copied().collect();
            for bus in frontier {
                for (id, _) in diagram.growth_options(&net, bus) {
                    diagram.grow(&net, bus, id);
                }
            }
        }
        assert!(diagram.placed.len() > 10, "growth should reach the network");

        let geometry = build(&net, &diagram, true);
        assert_eq!(geometry.buses.len(), diagram.placed.len());
        assert!(!geometry.elements.is_empty());
        for element in &geometry.elements {
            assert!(!element.legs.is_empty(), "{}", net.label(element.id));
            for leg in &element.legs {
                assert!(leg.points.len() >= 2);
                assert!(
                    leg.points
                        .iter()
                        .all(|p| p.x.is_finite() && p.y.is_finite())
                );
            }
        }
        let bounds = geometry.bounds().expect("a populated sheet has bounds");
        assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
    }

    #[test]
    fn grown_buses_never_land_on_top_of_each_other() {
        let net = sample_net();
        let mut diagram = Diagram::default();
        diagram.place(2, 0, 0);
        for (id, _) in diagram.growth_options(&net, 2) {
            diagram.grow(&net, 2, id);
        }
        for bus in diagram.placed.keys().copied().collect::<Vec<_>>() {
            for (id, _) in diagram.growth_options(&net, bus) {
                diagram.grow(&net, bus, id);
            }
        }
        let cells: Vec<(i32, i32)> = diagram.placed.values().map(|n| (n.gx, n.gy)).collect();
        let unique: std::collections::BTreeSet<(i32, i32)> = cells.iter().copied().collect();
        assert_eq!(cells.len(), unique.len(), "two buses share a grid cell");
    }

    #[test]
    fn hit_testing_finds_the_bar_and_the_run() {
        let net = sample_net();
        let mut diagram = Diagram::default();
        diagram.place(2, 0, 0);
        diagram.place(3, 0, 6);
        let geometry = build(&net, &diagram, false);
        assert_eq!(geometry.hit_bus(pos2(0.0, 0.0), 4.0), Some(2));
        assert_eq!(geometry.hit_bus(pos2(0.0, 3.0 * GRID), 4.0), None);
        let run = &geometry.elements[0].legs[0].points;
        assert!(geometry.hit_element(run[1], 4.0).is_some());
    }
}
