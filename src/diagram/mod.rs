//! The drawing: which buses have been placed, where, and what that makes visible.
//!
//! A raw file carries no coordinates, so the diagram is the part of the work the engineer owns.
//! It is kept separate from the [`Network`] so it can be saved and reloaded against the case.

pub mod layout;
pub mod render;
pub mod style;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use egui::{Pos2, Rect, Vec2, pos2};
use serde::{Deserialize, Serialize};

use crate::model::{ElemId, Network};
use style::GRID;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

impl Orientation {
    /// Unit vector along the bus bar.
    pub fn along(self) -> Vec2 {
        match self {
            Orientation::Horizontal => Vec2::X,
            Orientation::Vertical => Vec2::Y,
        }
    }

    /// The two directions connections may leave the bar in.
    pub fn sides(self) -> [Vec2; 2] {
        match self {
            Orientation::Horizontal => [Vec2::new(0.0, -1.0), Vec2::new(0.0, 1.0)],
            Orientation::Vertical => [Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0)],
        }
    }

    pub fn flipped(self) -> Orientation {
        match self {
            Orientation::Horizontal => Orientation::Vertical,
            Orientation::Vertical => Orientation::Horizontal,
        }
    }
}

/// A bus on the drawing, positioned in whole grid cells.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BusNode {
    pub gx: i32,
    pub gy: i32,
    #[serde(default)]
    pub orient: Orientation,
}

impl BusNode {
    pub fn center(&self) -> Pos2 {
        pos2(self.gx as f32 * GRID, self.gy as f32 * GRID)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Camera {
    /// World point shown at the centre of the viewport.
    pub cx: f32,
    pub cy: f32,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            cx: 0.0,
            cy: 0.0,
            zoom: 1.0,
        }
    }
}

impl Camera {
    pub fn center(&self) -> Pos2 {
        pos2(self.cx, self.cy)
    }

    pub fn to_screen(self, world: Pos2, viewport: Rect) -> Pos2 {
        viewport.center() + (world - self.center()) * self.zoom
    }

    pub fn to_world(self, screen: Pos2, viewport: Rect) -> Pos2 {
        self.center() + (screen - viewport.center()) / self.zoom
    }

    /// Frame `bounds` in the viewport, leaving a margin.
    pub fn fit(&mut self, bounds: Rect, viewport: Rect) {
        self.cx = bounds.center().x;
        self.cy = bounds.center().y;
        let margin = 2.0 * GRID;
        let sx = viewport.width() / (bounds.width() + margin * 2.0).max(1.0);
        let sy = viewport.height() / (bounds.height() + margin * 2.0).max(1.0);
        self.zoom = sx.min(sy).clamp(0.1, 4.0);
    }
}

/// Identifies an element by what it connects rather than by list index, so a saved drawing
/// still means the same thing after the case is read again.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ElemKey {
    pub transformer: bool,
    pub buses: Vec<i32>,
    pub ckt: String,
}

pub fn elem_key(net: &Network, id: ElemId) -> ElemKey {
    let element = net.element(id);
    ElemKey {
        transformer: matches!(id, ElemId::Transformer(_)),
        buses: element.terminals(),
        ckt: element.ckt().to_string(),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Diagram {
    pub placed: BTreeMap<i32, BusNode>,
    /// Elements the user has suppressed even though both ends are on the drawing.
    pub hidden: BTreeSet<ElemKey>,
    pub camera: Camera,
}

impl Diagram {
    pub fn is_placed(&self, bus: i32) -> bool {
        self.placed.contains_key(&bus)
    }

    pub fn place(&mut self, bus: i32, gx: i32, gy: i32) {
        self.placed.entry(bus).or_insert(BusNode {
            gx,
            gy,
            orient: Orientation::Horizontal,
        });
    }

    /// Take a bus off the drawing. Elements needing it simply stop being visible.
    pub fn remove(&mut self, bus: i32) {
        self.placed.remove(&bus);
    }

    pub fn clear(&mut self) {
        self.placed.clear();
        self.hidden.clear();
    }

    /// Elements with every terminal placed, and not suppressed.
    ///
    /// Drawing these automatically means closing a loop between two buses already on the sheet
    /// shows the tie without the user having to ask for it.
    pub fn visible_elements(&self, net: &Network) -> Vec<ElemId> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for bus in self.placed.keys() {
            for id in net.incident(*bus) {
                if !seen.insert(*id) {
                    continue;
                }
                let terminals = net.element(*id).terminals();
                if terminals.iter().all(|b| self.is_placed(*b))
                    && !self.hidden.contains(&elem_key(net, *id))
                {
                    out.push(*id);
                }
            }
        }
        out
    }

    /// Incident elements that would bring new buses onto the drawing, with those buses.
    pub fn growth_options(&self, net: &Network, bus: i32) -> Vec<(ElemId, Vec<i32>)> {
        net.incident(bus)
            .iter()
            .filter_map(|id| {
                let unplaced: Vec<i32> = net
                    .element(*id)
                    .terminals()
                    .into_iter()
                    .filter(|b| !self.is_placed(*b))
                    .collect();
                (!unplaced.is_empty()).then_some((*id, unplaced))
            })
            .collect()
    }

    /// Bring an element's far end(s) onto the drawing next to `from`.
    pub fn grow(&mut self, net: &Network, from: i32, id: ElemId) {
        let unplaced: Vec<i32> = net
            .element(id)
            .terminals()
            .into_iter()
            .filter(|b| !self.is_placed(*b))
            .collect();
        for bus in unplaced {
            let (gx, gy) = layout::find_free_slot(net, self, from, bus);
            self.place(bus, gx, gy);
        }
        self.hidden.remove(&elem_key(net, id));
    }
}

/// A saved drawing, paired with the case it was built from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub case: PathBuf,
    pub diagram: Diagram,
}

impl Project {
    pub fn save(path: &Path, case: &Path, diagram: &Diagram) -> std::io::Result<()> {
        let project = Project {
            case: case.to_path_buf(),
            diagram: diagram.clone(),
        };
        let json = serde_json::to_string_pretty(&project)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }

    pub fn load(path: &Path) -> std::io::Result<Project> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psse;

    fn sample_net() -> Network {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("cases/MemphisCase2026_Mar7.RAW");
        Network::from_raw(psse::parse_file(&path).expect("sample case parses"))
    }

    #[test]
    fn an_element_appears_once_both_ends_are_placed() {
        let net = sample_net();
        let mut diagram = Diagram::default();
        diagram.place(2, 0, 0);
        assert!(diagram.visible_elements(&net).is_empty());
        diagram.place(3, 0, 4);
        let visible: Vec<String> = diagram
            .visible_elements(&net)
            .iter()
            .map(|id| net.label(*id))
            .collect();
        assert_eq!(visible, vec!["2-3 xfmr 1"]);
    }

    #[test]
    fn growing_places_the_far_bus_clear_of_the_near_one() {
        let net = sample_net();
        let mut diagram = Diagram::default();
        diagram.place(2, 0, 0);
        let (id, unplaced) = diagram
            .growth_options(&net, 2)
            .into_iter()
            .find(|(id, _)| net.label(*id) == "2-722 line 1")
            .expect("bus 2 has a line to 722");
        assert_eq!(unplaced, vec![722]);
        diagram.grow(&net, 2, id);
        let node = diagram.placed[&722];
        assert!((node.gx, node.gy) != (0, 0));
        assert_eq!(diagram.visible_elements(&net).len(), 1);
    }

    #[test]
    fn hidden_elements_stay_off_the_drawing() {
        let net = sample_net();
        let mut diagram = Diagram::default();
        diagram.place(2, 0, 0);
        diagram.place(3, 0, 4);
        let id = diagram.visible_elements(&net)[0];
        diagram.hidden.insert(elem_key(&net, id));
        assert!(diagram.visible_elements(&net).is_empty());
    }

    #[test]
    fn a_drawing_round_trips_through_a_file_on_disk() {
        let mut diagram = Diagram::default();
        diagram.place(2, 4, -1);
        diagram.place(722, -12, 3);
        diagram.hidden.insert(ElemKey {
            transformer: true,
            buses: vec![2, 3],
            ckt: "1".to_string(),
        });
        diagram.camera = Camera {
            cx: 12.0,
            cy: -8.0,
            zoom: 2.5,
        };

        let path = std::env::temp_dir().join("pf-vis-round-trip.json");
        Project::save(&path, Path::new("case.raw"), &diagram).expect("saves");
        let back = Project::load(&path).expect("loads");
        let _ = std::fs::remove_file(&path);

        assert_eq!(back.case, PathBuf::from("case.raw"));
        assert_eq!(back.diagram.placed.len(), 2);
        assert_eq!(
            (back.diagram.placed[&722].gx, back.diagram.placed[&722].gy),
            (-12, 3)
        );
        assert_eq!(back.diagram.hidden.len(), 1);
        assert_eq!(back.diagram.camera.zoom, 2.5);
    }

    #[test]
    fn a_project_round_trips_through_json() {
        let mut diagram = Diagram::default();
        diagram.place(2, 1, -3);
        diagram.camera.zoom = 1.75;
        let json = serde_json::to_string(&Project {
            case: PathBuf::from("case.raw"),
            diagram,
        })
        .unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.diagram.placed[&2].gy, -3);
        assert_eq!(back.diagram.camera.zoom, 1.75);
    }
}


