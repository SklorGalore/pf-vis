//! The application: a case on the left, the drawing in the middle, what is selected on the right.

use std::path::{Path, PathBuf};

use eframe::egui;
use egui::{Align, Color32, Layout, RichText, Sense, Vec2};

use crate::diagram::layout;
use crate::diagram::render::{self, Pick};
use crate::diagram::style::{self, GRID, format_kv, voltage_color};
use crate::diagram::{Diagram, Project, elem_key};
use crate::model::{ElemId, Element, Network};
use crate::psse;

pub struct PfVisApp {
    net: Option<Network>,
    case_path: Option<PathBuf>,
    project_path: Option<PathBuf>,
    diagram: Diagram,
    status: String,
    filter: String,
    selected: Option<Pick>,
    hovered: Option<Pick>,
    show_grid: bool,
    show_labels: bool,
    show_equipment: bool,
    /// Bus currently being dragged, and the grab offset in world units.
    dragging: Option<(i32, Vec2)>,
    fit_requested: bool,
}

impl Default for PfVisApp {
    fn default() -> Self {
        PfVisApp {
            net: None,
            case_path: None,
            project_path: None,
            diagram: Diagram::default(),
            status: "Open a PSS/E raw file to begin.".to_string(),
            filter: String::new(),
            selected: None,
            hovered: None,
            show_grid: true,
            show_labels: true,
            show_equipment: true,
            dragging: None,
            fit_requested: false,
        }
    }
}

impl PfVisApp {
    /// Open whatever was named on the command line: a saved drawing, or a case to start from.
    pub fn new(open: Option<PathBuf>) -> Self {
        let mut app = PfVisApp::default();
        match open {
            Some(path)
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("json")) =>
            {
                app.open_project(&path)
            }
            Some(path) => app.open_case(&path),
            None => {}
        }
        app
    }

    fn open_case(&mut self, path: &Path) {
        match psse::parse_file(path) {
            Ok(case) => {
                let net = Network::from_raw(case);
                self.status = format!(
                    "{}: {} buses, {} branches, {} transformers",
                    path.display(),
                    net.buses.len(),
                    net.branches.len(),
                    net.transformers.len()
                );
                self.net = Some(net);
                self.case_path = Some(path.to_path_buf());
                self.diagram = Diagram::default();
                self.selected = None;
            }
            Err(e) => self.status = format!("{}: {e}", path.display()),
        }
    }

    fn open_project(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(project) => {
                // The drawing is meaningless without its case, so load that first.
                let case = if project.case.exists() {
                    project.case.clone()
                } else {
                    // Drawings are often moved alongside their case.
                    path.with_file_name(
                        project.case.file_name().unwrap_or(project.case.as_os_str()),
                    )
                };
                self.open_case(&case);
                if self.net.is_some() {
                    self.diagram = project.diagram;
                    // A saved drawing keeps its own view, unless the file has a nonsense one.
                    self.fit_requested =
                        !(self.diagram.camera.zoom.is_finite() && self.diagram.camera.zoom > 0.0);
                    self.project_path = Some(path.to_path_buf());
                    self.status = format!(
                        "{}: {} buses on the drawing",
                        path.display(),
                        self.diagram.placed.len()
                    );
                }
            }
            Err(e) => self.status = format!("{}: {e}", path.display()),
        }
    }

    fn save_project(&mut self, path: &Path) {
        let Some(case) = self.case_path.clone() else {
            self.status = "Nothing to save: no case is open.".to_string();
            return;
        };
        match Project::save(path, &case, &self.diagram) {
            Ok(()) => {
                self.project_path = Some(path.to_path_buf());
                self.status = format!("Saved {}", path.display());
            }
            Err(e) => self.status = format!("{}: {e}", path.display()),
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Open case…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("PSS/E raw", &["raw", "RAW"])
                    .pick_file()
            {
                self.open_case(&path);
            }
            if ui.button("Open drawing…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("pf-vis drawing", &["json"])
                    .pick_file()
            {
                self.open_project(&path);
            }
            if ui
                .add_enabled(self.case_path.is_some(), egui::Button::new("Save drawing…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("pf-vis drawing", &["json"])
                    .set_file_name("drawing.json")
                    .save_file()
            {
                self.save_project(&path);
            }
            ui.separator();
            if ui.button("Fit").clicked() {
                self.fit_requested = true;
            }
            if ui
                .add_enabled(
                    !self.diagram.placed.is_empty(),
                    egui::Button::new("Clear drawing"),
                )
                .clicked()
            {
                self.diagram.clear();
                self.selected = None;
            }
            ui.separator();
            ui.checkbox(&mut self.show_grid, "Grid");
            ui.checkbox(&mut self.show_labels, "Labels");
            ui.checkbox(&mut self.show_equipment, "Equipment");
        });
    }

    fn case_panel(&mut self, ui: &mut egui::Ui) {
        let Some(net) = &self.net else {
            ui.label("No case open.");
            return;
        };

        egui::CollapsingHeader::new("Case")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("case-summary")
                    .num_columns(2)
                    .show(ui, |ui| {
                        let mut row = |k: &str, v: String| {
                            ui.label(k);
                            ui.label(v);
                            ui.end_row();
                        };
                        row("revision", net.rev.to_string());
                        row("base MVA", format!("{:.0}", net.sbase));
                        row("buses", net.buses.len().to_string());
                        row("branches", net.branches.len().to_string());
                        row("transformers", net.transformers.len().to_string());
                        row("generators", net.generators.len().to_string());
                        row("loads", net.loads.len().to_string());
                        row(
                            "shunts",
                            (net.fixed_shunts.len() + net.switched_shunts.len()).to_string(),
                        );
                    });
                if net.change_case {
                    ui.label(
                        RichText::new(
                            "This is a change case (IC = 1): its records modify another case, \
                             so the network read here is only part of a system.",
                        )
                        .small()
                        .color(Color32::from_rgb(0xB0, 0x50, 0x00)),
                    );
                }
                if net.dangling > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} records reference buses this case does not define and were \
                             dropped",
                            net.dangling
                        ))
                        .small()
                        .color(Color32::from_rgb(0xB0, 0x50, 0x00)),
                    );
                }
                if !net.skipped.is_empty() {
                    let names: Vec<String> = net
                        .skipped
                        .iter()
                        .map(|(name, n)| format!("{name} ({n})"))
                        .collect();
                    ui.label(
                        RichText::new(format!("not read: {}", names.join(", ")))
                            .small()
                            .color(Color32::GRAY),
                    );
                }
            });

        egui::CollapsingHeader::new("Voltage levels")
            .default_open(true)
            .show(ui, |ui| {
                for kv in net.voltage_levels() {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(18.0, 6.0), Sense::hover());
                        ui.painter().rect_filled(rect, 1.0, voltage_color(kv));
                        ui.label(format_kv(kv));
                    });
                }
            });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Find bus");
            ui.text_edit_singleline(&mut self.filter);
        });

        let matches: Vec<i32> = net
            .buses
            .values()
            .filter(|b| matches_filter(b.number, &b.name, b.base_kv, &self.filter))
            .map(|b| b.number)
            .collect();
        ui.label(
            RichText::new(format!("{} of {} buses", matches.len(), net.buses.len()))
                .small()
                .color(Color32::GRAY),
        );

        let row_height = ui.spacing().interact_size.y;
        let mut to_place = None;
        egui::ScrollArea::vertical().show_rows(ui, row_height, matches.len(), |ui, range| {
            for number in &matches[range] {
                let bus = &net.buses[number];
                let placed = self.diagram.is_placed(*number);
                ui.horizontal(|ui| {
                    let mark = if placed { "•" } else { " " };
                    let text = RichText::new(format!(
                        "{mark} {} {}  {}",
                        bus.number,
                        bus.name,
                        format_kv(bus.base_kv)
                    ))
                    .color(voltage_color(bus.base_kv));
                    if ui
                        .add_enabled(!placed, egui::Button::new(text).frame(false))
                        .clicked()
                    {
                        to_place = Some(*number);
                    }
                });
            }
        });

        if let Some(bus) = to_place {
            let anchor = (
                (self.diagram.camera.cx / GRID).round() as i32,
                (self.diagram.camera.cy / GRID).round() as i32,
            );
            let (gx, gy) = layout::free_cell_near(&self.diagram, anchor);
            let first = self.diagram.placed.is_empty();
            self.diagram.place(bus, gx, gy);
            self.selected = Some(Pick::Bus(bus));
            self.fit_requested |= first;
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let Some(net) = &self.net else { return };
        match self.selected {
            None => {
                ui.label("Nothing selected.");
                ui.label(
                    RichText::new(
                        "Pick a bus from the list to place it, then select it here to grow the \
                         drawing along its connections.",
                    )
                    .small()
                    .color(Color32::GRAY),
                );
            }
            Some(Pick::Bus(bus)) => self.bus_inspector(ui, bus),
            Some(Pick::Element(id)) => {
                ui.heading(net.label(id));
                match net.element(id) {
                    Element::Branch(branch) => {
                        record_grid(
                            ui,
                            "branch",
                            vec![
                                row("from", branch.from.to_string()),
                                row("to", branch.to.to_string()),
                                row("circuit", branch.ckt.clone()),
                                row("name", branch.name.clone()),
                                row("R pu", format!("{:.5}", branch.r)),
                                row("X pu", format!("{:.5}", branch.x)),
                                row("B pu", format!("{:.5}", branch.b)),
                                row("rating A", format!("{:.1} MVA", branch.rate_a)),
                                row("status", in_service(branch.status)),
                            ],
                        );
                    }
                    Element::Transformer(xf) => {
                        let mut rows = vec![
                            row("winding 1 bus", xf.i.to_string()),
                            row("winding 2 bus", xf.j.to_string()),
                        ];
                        if xf.is_three_winding() {
                            rows.push(row("winding 3 bus", xf.k.to_string()));
                        }
                        rows.extend([
                            row("circuit", xf.ckt.clone()),
                            row("name", xf.name.clone()),
                            row("R 1-2 pu", format!("{:.5}", xf.r12)),
                            row("X 1-2 pu", format!("{:.5}", xf.x12)),
                            row("status", in_service(xf.status)),
                        ]);
                        for (i, w) in xf.windings.iter().enumerate() {
                            rows.push(row(
                                &format!("winding {}", i + 1),
                                format!(
                                    "{:.4} pu tap, {} nominal, {:.2}°, {:.1} MVA",
                                    w.windv,
                                    format_kv(w.nomv),
                                    w.ang,
                                    w.rate_a
                                ),
                            ));
                        }
                        record_grid(ui, "transformer", rows);
                    }
                }
                ui.separator();
                if ui.button("Hide from drawing").clicked() {
                    self.diagram.hidden.insert(elem_key(net, id));
                    self.selected = None;
                }
            }
        }
    }

    fn bus_inspector(&mut self, ui: &mut egui::Ui, bus: i32) {
        let Some(net) = &self.net else { return };
        let Some(record) = net.bus(bus) else { return };

        ui.heading(format!("{bus} {}", record.name));
        record_grid(
            ui,
            "bus",
            vec![
                row("nominal", format_kv(record.base_kv)),
                row("type", bus_type(record.ide)),
                row("area", record.area.to_string()),
                row("zone", record.zone.to_string()),
                row(
                    "solved",
                    format!("{:.4} pu at {:.2} deg", record.vm, record.va),
                ),
            ],
        );

        if let Some(attached) = net.attached(bus)
            && attached.count() > 0
        {
            ui.separator();
            ui.label(RichText::new("Equipment").strong());
            for i in &attached.generators {
                let g = &net.generators[*i];
                ui.label(format!(
                    "generator {} · {:.1} MW, {:.1} Mvar on {:.0} MVA",
                    g.id, g.pg, g.qg, g.mbase
                ));
            }
            for i in &attached.loads {
                let l = &net.loads[*i];
                ui.label(format!("load {} · {:.1} MW, {:.1} Mvar", l.id, l.pl, l.ql));
            }
            for i in &attached.fixed_shunts {
                let s = &net.fixed_shunts[*i];
                ui.label(format!(
                    "fixed shunt {} · {} · {:.1} MW, {:.1} Mvar at 1 pu",
                    s.id,
                    shunt_word(s.bl),
                    s.gl,
                    s.bl
                ));
            }
            for i in &attached.switched_shunts {
                let s = &net.switched_shunts[*i];
                ui.label(format!(
                    "switched shunt {} · {} · {:.1} Mvar at 1 pu",
                    s.id,
                    shunt_word(s.binit),
                    s.binit
                ));
            }
        }

        ui.separator();
        ui.label(RichText::new("Connections").strong());

        let mut action: Option<(ElemId, bool)> = None;
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                let growable: std::collections::BTreeMap<ElemId, Vec<i32>> =
                    self.diagram.growth_options(net, bus).into_iter().collect();
                for id in net.incident(bus) {
                    let element = net.element(*id);
                    let far: Vec<i32> = element
                        .terminals()
                        .into_iter()
                        .filter(|b| *b != bus)
                        .collect();
                    let on_drawing = !growable.contains_key(id);
                    ui.horizontal(|ui| {
                        if on_drawing {
                            ui.add_enabled(false, egui::Button::new("on sheet"));
                        } else if ui.button("Add").clicked() {
                            action = Some((*id, true));
                        }
                        let far_names: Vec<String> = far
                            .iter()
                            .map(|b| match net.bus(*b) {
                                Some(r) => format!("{b} {}", r.name),
                                None => b.to_string(),
                            })
                            .collect();
                        let kind = match element {
                            Element::Branch(_) => "line",
                            Element::Transformer(t) if t.is_three_winding() => "3-w xfmr",
                            Element::Transformer(t) if t.is_phase_shifter() => "phase shifter",
                            Element::Transformer(_) => "xfmr",
                        };
                        let color = net
                            .bus(*far.first().unwrap_or(&bus))
                            .map_or(style::INK, |r| voltage_color(r.base_kv));
                        let mut text = RichText::new(format!(
                            "{kind} {} → {}",
                            element.ckt(),
                            far_names.join(", ")
                        ))
                        .color(color);
                        if !element.in_service() {
                            text = text.italics();
                        }
                        if ui.selectable_label(false, text).clicked() && on_drawing {
                            action = Some((*id, false));
                        }
                    });
                }
            });

        if let Some((id, grow)) = action {
            let net = self.net.as_ref().expect("case is open");
            if grow {
                self.diagram.grow(net, bus, id);
            } else {
                self.selected = Some(Pick::Element(id));
            }
        }

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Rotate").clicked()
                && let Some(node) = self.diagram.placed.get_mut(&bus)
            {
                node.orient = node.orient.flipped();
            }
            if ui.button("Remove from drawing").clicked() {
                self.diagram.remove(bus);
                self.selected = None;
            }
        });
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let Some(net) = &self.net else {
            ui.centered_and_justified(|ui| {
                ui.label("Open a PSS/E raw file, then place a bus to start the drawing.");
            });
            return;
        };

        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let viewport = response.rect;
        let geometry = layout::build(net, &self.diagram, self.show_equipment);

        if self.fit_requested {
            self.fit_requested = false;
            if let Some(bounds) = geometry.bounds() {
                self.diagram.camera.fit(bounds, viewport);
            }
        }

        // Zoom about the pointer so the thing being examined stays put.
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0
                && let Some(pointer) = response.hover_pos()
            {
                let before = self.diagram.camera.to_world(pointer, viewport);
                let zoom = (self.diagram.camera.zoom * (1.0 + scroll * 0.0015)).clamp(0.08, 6.0);
                self.diagram.camera.zoom = zoom;
                let after = self.diagram.camera.to_world(pointer, viewport);
                self.diagram.camera.cx -= after.x - before.x;
                self.diagram.camera.cy -= after.y - before.y;
            }
        }

        let pointer_world = response
            .hover_pos()
            .map(|p| self.diagram.camera.to_world(p, viewport));
        let tolerance = 7.0 / self.diagram.camera.zoom;
        self.hovered = pointer_world.and_then(|world| {
            geometry
                .hit_bus(world, tolerance.max(GRID * 0.15))
                .map(Pick::Bus)
                .or_else(|| geometry.hit_element(world, tolerance).map(Pick::Element))
        });

        if response.drag_started()
            && let (Some(world), Some(Pick::Bus(bus))) = (pointer_world, self.hovered)
        {
            let node = self.diagram.placed[&bus];
            self.dragging = Some((bus, node.center() - world));
        }
        if response.dragged() {
            match self.dragging {
                // Dragging a bus moves it, snapped to the grid it was placed on.
                Some((bus, grab)) => {
                    if let (Some(world), Some(node)) =
                        (pointer_world, self.diagram.placed.get_mut(&bus))
                    {
                        let target = world + grab;
                        node.gx = (target.x / GRID).round() as i32;
                        node.gy = (target.y / GRID).round() as i32;
                    }
                }
                // Dragging anywhere else pans the sheet.
                None => {
                    let delta = response.drag_delta() / self.diagram.camera.zoom;
                    self.diagram.camera.cx -= delta.x;
                    self.diagram.camera.cy -= delta.y;
                }
            }
        }
        if response.drag_stopped() {
            self.dragging = None;
        }
        if response.clicked() {
            self.selected = self.hovered;
        }

        render::draw(
            &painter,
            viewport,
            self.diagram.camera,
            net,
            &geometry,
            &render::Options {
                show_grid: self.show_grid,
                show_labels: self.show_labels,
                selected: self.selected,
                hovered: self.hovered,
            },
        );
    }
}

impl eframe::App for PfVisApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).small());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} buses on sheet", self.diagram.placed.len()))
                            .small()
                            .color(Color32::GRAY),
                    );
                });
            });
        });
        egui::Panel::left("case")
            .default_size(300.0)
            .show(ui, |ui| self.case_panel(ui));
        egui::Panel::right("inspector")
            .default_size(320.0)
            .show(ui, |ui| self.inspector(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));
    }
}

/// Match on bus number, name or nominal voltage, so one box covers every way of looking.
fn matches_filter(number: i32, name: &str, base_kv: f64, filter: &str) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return true;
    }
    let lower = filter.to_ascii_lowercase();
    if name.to_ascii_lowercase().contains(&lower) || number.to_string().contains(&lower) {
        return true;
    }
    match lower.trim_end_matches("kv").trim().parse::<f64>() {
        Ok(kv) => (base_kv - kv).abs() < 0.05,
        Err(_) => false,
    }
}

fn record_grid(ui: &mut egui::Ui, id: &str, rows: Vec<(String, String)>) {
    egui::Grid::new(id)
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for (key, value) in rows {
                ui.label(RichText::new(key).color(Color32::GRAY));
                ui.label(value);
                ui.end_row();
            }
        });
}

/// Shorthand for one row of a record table, so the tables below stay readable.
fn row(key: &str, value: impl Into<String>) -> (String, String) {
    (key.to_string(), value.into())
}

fn in_service(status: i32) -> String {
    if status == 0 {
        "out of service".to_string()
    } else {
        "in service".to_string()
    }
}

fn bus_type(ide: i32) -> &'static str {
    match ide {
        2 => "generator (PV)",
        3 => "swing",
        4 => "isolated",
        _ => "load (PQ)",
    }
}

fn shunt_word(b: f64) -> &'static str {
    if b < 0.0 { "reactor" } else { "capacitor" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_box_matches_number_name_and_voltage() {
        assert!(matches_filter(2, "Greers Fer~1", 138.0, ""));
        assert!(matches_filter(2, "Greers Fer~1", 138.0, "greers"));
        assert!(matches_filter(2, "Greers Fer~1", 138.0, "138 kV"));
        assert!(matches_filter(722, "Marion", 69.0, "72"));
        assert!(!matches_filter(2, "Greers Fer~1", 138.0, "marion"));
        assert!(!matches_filter(2, "Greers Fer~1", 138.0, "345"));
    }
}
