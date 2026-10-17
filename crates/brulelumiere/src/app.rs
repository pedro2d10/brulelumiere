//! The BrûleLumière application: an eframe GUI over the typed [`Project`] model,
//! persisted through [`bl_io::ConfigStore`] with LightBurn-style
//! atomic save, rotating backups and a version guard.

use std::path::PathBuf;

use eframe::egui;
use bl_io::{ConfigStore, Error as IoError};

use crate::model::{CutLayer, Device, LayerType, Origin, Project, Units};
use crate::shapes::{layer_rgb, Prim, Shape, ShapeKind, Vertex, XForm};

const APP_VERSION: u64 = 1;

/// The active drawing tool, mirroring LightBurn's Select / Rectangle / Ellipse
/// / Draw-Lines tools.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Tool {
    Select,
    Rect,
    Ellipse,
    Polygon,
    Pen,
    Node,
    Measure,
}

impl Tool {
    fn label(self) -> &'static str {
        match self {
            Tool::Select => "Sélection",
            Tool::Rect => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Polygon => "Polygone",
            Tool::Pen => "Lignes (plume)",
            Tool::Node => "Nœuds",
            Tool::Measure => "Mesure",
        }
    }

    /// A single-glyph icon for the left tool palette (LightBurn-style).
    fn icon(self) -> &'static str {
        match self {
            Tool::Select => "⬏",
            Tool::Rect => "▭",
            Tool::Ellipse => "◯",
            Tool::Polygon => "⬠",
            Tool::Pen => "✐",
            Tool::Node => "✎",
            Tool::Measure => "📏",
        }
    }
}

/// Which floating tool windows are currently open (LightBurn's dockable
/// windows: cut settings, G-code preview, laser console, device settings…).
#[derive(Default)]
struct Windows {
    cut_settings: bool,
    gcode: bool,
    machine: bool,
    devices: bool,
    prefs: bool,
    about: bool,
    camera: bool,
}

pub struct BruleLumiereApp {
    store: ConfigStore,
    project: Project,
    windows: Windows,
    status: String,
    last_backup: Option<PathBuf>,

    // --- canvas state ---
    tool: Tool,
    /// Cut layer new shapes are assigned to.
    active_layer: u32,
    /// Pixels per model-mm for the canvas view.
    px_per_mm: f32,
    /// Drag anchor (model coords) for rect/ellipse rubber-banding.
    drag_start: Option<(f64, f64)>,
    /// Points accumulated by the pen tool (model coords).
    pending_pen: Vec<(f64, f64)>,
    /// Index of the currently selected shape (primary, for single-shape ops).
    selected: Option<usize>,
    /// Multi-selection for transforms/grouping (Select tool, Ctrl+click).
    sel: Vec<usize>,
    /// Index of the selected vertex within the selected path (node edit tool).
    selected_node: Option<usize>,
    /// Snap moves to a 1 mm grid.
    snap: bool,
    /// True while dragging a shape body with the Select tool.
    moving: bool,
    /// Pan offset (screen px) added to the canvas origin.
    view_offset: egui::Vec2,
    /// The canvas rectangle from the last frame (for zoom-to-fit maths).
    last_canvas: egui::Rect,
    /// Number of sides for the polygon tool.
    poly_sides: u32,
    /// First / second click of the measure tool (model coords).
    measure_a: Option<(f64, f64)>,
    measure_b: Option<(f64, f64)>,
    /// Last measured distance in mm (for the status overlay).
    measure_mm: Option<f64>,
    /// Offset distance (mm) used by the Offset / Dilate / Contract actions.
    offset_dist: f64,
    /// Internal clipboard for cut/copy/paste/duplicate (world-space shapes).
    clipboard: Vec<Shape>,
    /// Image-trace (camera) options and source path.
    trace_opts: crate::trace::TraceOptions,
    trace_path: String,
    /// Undo / redo stacks of whole-project snapshots (LightBurn-style history).
    undo_stack: Vec<Project>,
    redo_stack: Vec<Project>,
    /// Path used by the import/export buttons.
    io_path: String,
    /// G-code generation options and last generated job.
    gopts: crate::gcode::GcodeOptions,
    job: Option<crate::gcode::Job>,

    // --- text & array tools ---
    text_input: String,
    text_size: f64,
    array_cols: u32,
    array_rows: u32,
    array_dx: f64,
    array_dy: f64,

    // --- machine connection ---
    machine: Option<crate::machine::Machine>,
    ports: Vec<String>,
    port_sel: String,
    baud: u32,
    jog_step: f64,
    jog_feed: f64,
}

impl BruleLumiereApp {
    /// Open (or initialize) the store under `base_dir` and load the project.
    pub fn new(base_dir: PathBuf) -> Self {
        let defaults = serde_json::to_value(Project::default())
            .expect("default project serializes");

        let mut store = ConfigStore::new(base_dir)
            .with_file_name("project.json")
            .with_version(APP_VERSION, "__saveVersion")
            .with_max_backups(10)
            .with_defaults(defaults);

        let (project, status) = match store.load() {
            Ok(()) => match serde_json::from_value::<Project>(store.document().clone()) {
                Ok(p) => (p, format!("Chargé : {}", store.path().display())),
                Err(e) => (
                    Project::default(),
                    format!("Document illisible ({e}) — projet par défaut chargé"),
                ),
            },
            Err(IoError::NewerVersion { file_version, app_version, .. }) => (
                Project::default(),
                format!(
                    "Fichier écrit par une version plus récente (save {file_version} > {app_version}) — \
                     non chargé, projet par défaut à la place"
                ),
            ),
            Err(e) => (Project::default(), format!("Échec du chargement : {e}")),
        };

        let io_path = store.path().with_file_name("export.lbrn2").display().to_string();
        Self {
            store,
            project,
            windows: Windows::default(),
            status,
            last_backup: None,
            tool: Tool::Rect,
            active_layer: 0,
            px_per_mm: 2.0,
            drag_start: None,
            pending_pen: Vec::new(),
            selected: None,
            sel: Vec::new(),
            selected_node: None,
            snap: false,
            moving: false,
            view_offset: egui::Vec2::ZERO,
            last_canvas: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(700.0, 500.0)),
            poly_sides: 6,
            measure_a: None,
            measure_b: None,
            measure_mm: None,
            offset_dist: 2.0,
            clipboard: Vec::new(),
            trace_opts: crate::trace::TraceOptions::default(),
            trace_path: String::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            io_path,
            gopts: crate::gcode::GcodeOptions::default(),
            job: None,
            text_input: "BRULE".to_string(),
            text_size: 20.0,
            array_cols: 3,
            array_rows: 2,
            array_dx: 30.0,
            array_dy: 30.0,
            machine: None,
            ports: crate::machine::list_ports(),
            port_sel: String::new(),
            baud: 115200,
            jog_step: 10.0,
            jog_feed: 3000.0,
        }
    }

    // ------------------------------------------------------------------
    // Machine connection (#7): serial GRBL, job streaming, jog, frame.
    // ------------------------------------------------------------------

    fn machine_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("🔄 Rafraîchir les ports").clicked() {
                self.ports = crate::machine::list_ports();
                if self.port_sel.is_empty() {
                    self.port_sel = self.ports.first().cloned().unwrap_or_default();
                }
            }
            egui::ComboBox::from_id_salt("port")
                .selected_text(if self.port_sel.is_empty() { "— port —".to_string() } else { self.port_sel.clone() })
                .show_ui(ui, |ui| {
                    for p in &self.ports {
                        ui.selectable_value(&mut self.port_sel, p.clone(), p);
                    }
                });
            ui.label("Baud :");
            ui.add(egui::DragValue::new(&mut self.baud).range(9600..=1_000_000).speed(100.0));

            let connected = self.machine.is_some();
            if !connected {
                if ui.add_enabled(!self.port_sel.is_empty(), egui::Button::new("🔌 Connecter")).clicked() {
                    match crate::machine::Machine::connect(&self.port_sel, self.baud) {
                        Ok(m) => {
                            self.machine = Some(m);
                            self.status = format!("Connecté à {}", self.port_sel);
                        }
                        Err(e) => self.status = e,
                    }
                }
            } else if ui.button("⏏ Déconnecter").clicked() {
                self.machine = None; // Drop sends Disconnect and joins the worker.
                self.status = "Machine déconnectée".into();
            }
        });
        ui.separator();

        if self.machine.is_none() {
            ui.weak("Connecte une machine GRBL (ex. /dev/ttyUSB0) pour envoyer un job, encadrer ou déplacer la tête.");
            ui.weak("⚠ Non testé sans matériel : l'envoi déclenche réellement le laser.");
            return;
        }

        use crate::machine::Cmd;
        // Collect the frame's commands and deferred state, so we never hold a
        // borrow of `self.machine` while mutating `self`.
        let mut cmds: Vec<Cmd> = Vec::new();
        let mut new_status: Option<String> = None;
        let mut cache_job: Option<crate::gcode::Job> = None;

        ui.horizontal_wrapped(|ui| {
            if ui.button("▶ Envoyer le job").clicked() {
                let job = self
                    .job
                    .clone()
                    .unwrap_or_else(|| crate::gcode::generate(&self.project, &self.gopts));
                cmds.push(Cmd::Job(job.gcode.lines().map(|l| l.to_string()).collect()));
                cache_job = Some(job);
            }
            if ui.button("⬛ Cadre").clicked() {
                match self.project_bbox() {
                    Some(bbox) => cmds.push(Cmd::Job(crate::machine::frame_lines(bbox, self.jog_feed))),
                    None => new_status = Some("Rien à encadrer".into()),
                }
            }
            if ui.button("⏸ Pause (!)").clicked() {
                cmds.push(Cmd::FeedHold);
            }
            if ui.button("▶ Reprendre (~)").clicked() {
                cmds.push(Cmd::Resume);
            }
            if ui.button("🏠 Origine").clicked() {
                cmds.push(Cmd::Line("G90 G0 X0 Y0".into()));
            }
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Pas :");
            ui.add(egui::DragValue::new(&mut self.jog_step).range(0.1..=200.0).suffix(" mm"));
            ui.label("Vitesse :");
            ui.add(egui::DragValue::new(&mut self.jog_feed).range(100.0..=10000.0).suffix(" mm/min"));
        });
        let (step, feed) = (self.jog_step, self.jog_feed);
        egui::Grid::new("jog").spacing([6.0, 6.0]).show(ui, |ui| {
            ui.label("");
            if ui.button("⬆").clicked() {
                cmds.push(Cmd::Jog { dx: 0.0, dy: step, feed });
            }
            ui.label("");
            ui.end_row();
            if ui.button("⬅").clicked() {
                cmds.push(Cmd::Jog { dx: -step, dy: 0.0, feed });
            }
            ui.label("•");
            if ui.button("➡").clicked() {
                cmds.push(Cmd::Jog { dx: step, dy: 0.0, feed });
            }
            ui.end_row();
            ui.label("");
            if ui.button("⬇").clicked() {
                cmds.push(Cmd::Jog { dx: 0.0, dy: -step, feed });
            }
            ui.label("");
            ui.end_row();
        });

        // Single borrow of the machine: dispatch queued commands, read status.
        let (sending, sent, total, log) = {
            let m = self.machine.as_ref().unwrap();
            for c in cmds {
                m.send(c);
            }
            let (_connected, sending, sent, total, log) = m.status();
            (sending, sent, total, log)
        };

        ui.separator();
        if sending || total > 0 {
            ui.label(format!("Progression : {sent}/{total} lignes"));
            if total > 0 {
                ui.add(egui::ProgressBar::new(sent as f32 / total as f32).desired_width(300.0));
            }
        }
        egui::ScrollArea::vertical().max_height(200.0).stick_to_bottom(true).show(ui, |ui| {
            for line in &log {
                ui.monospace(line);
            }
        });

        if let Some(s) = new_status {
            self.status = s;
        }
        if let Some(j) = cache_job {
            self.job = Some(j);
        }
        // Keep the log/progress live while connected.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }

    /// Combined world bounding box of all shapes.
    fn project_bbox(&self) -> Option<(f64, f64, f64, f64)> {
        let mut bb: Option<(f64, f64, f64, f64)> = None;
        for s in &self.project.shapes {
            if let Some((a, b, c, d)) = s.world_bbox(48) {
                bb = Some(match bb {
                    None => (a, b, c, d),
                    Some((e, f, g, h)) => (e.min(a), f.min(b), g.max(c), h.max(d)),
                });
            }
        }
        bb
    }

    // ------------------------------------------------------------------
    // Text, array and boolean tools (#6).
    // ------------------------------------------------------------------

    /// Insert the current text as a group of vector stroke paths.
    fn insert_text(&mut self) {
        if self.text_input.is_empty() {
            return;
        }
        self.push_undo();
        let strokes = crate::text::vectorize(&self.text_input, self.text_size);
        let children: Vec<Shape> = strokes
            .iter()
            .map(|poly| {
                let offset: Vec<(f64, f64)> = poly.iter().map(|&(x, y)| (x + 10.0, y + 10.0)).collect();
                Shape::polyline(self.active_layer, &offset, false)
            })
            .collect();
        if children.is_empty() {
            return;
        }
        let group = Shape {
            cut_index: self.active_layer,
            xform: XForm::IDENTITY,
            kind: ShapeKind::Group { children },
        };
        self.project.shapes.push(group);
        let gi = self.project.shapes.len() - 1;
        self.sel = vec![gi];
        self.selected = Some(gi);
        self.status = format!("Texte « {} » vectorisé", self.text_input);
    }

    /// Duplicate the selection into a rows×cols grid (LightBurn's grid array).
    fn array_selection(&mut self) {
        let base: Vec<Shape> = self.sel.iter().filter_map(|&i| self.project.shapes.get(i).cloned()).collect();
        if base.is_empty() {
            return;
        }
        self.push_undo();
        let mut added = 0;
        for row in 0..self.array_rows {
            for col in 0..self.array_cols {
                if row == 0 && col == 0 {
                    continue; // keep the originals in place
                }
                let off = XForm::translate(col as f64 * self.array_dx, row as f64 * self.array_dy);
                for s in &base {
                    let mut clone = s.clone();
                    clone.xform = off.compose(&clone.xform);
                    self.project.shapes.push(clone);
                    added += 1;
                }
            }
        }
        self.status = format!("Tableau : {added} copie(s) ajoutée(s)");
    }

    /// Apply a boolean operation to the first two selected shapes.
    fn boolean_selection(&mut self, op: crate::boolean::BoolOp) {
        if self.sel.len() < 2 {
            return;
        }
        let (ia, ib) = (self.sel[0], self.sel[1]);
        let (Some(a), Some(b)) = (self.project.shapes.get(ia).cloned(), self.project.shapes.get(ib).cloned())
        else {
            return;
        };
        let result = crate::boolean::apply(&a, &b, op, self.active_layer);
        if result.is_empty() {
            self.status = "Opération booléenne : formes fermées requises".into();
            return;
        }
        self.push_undo();
        // Remove the two inputs (highest index first) and append the result.
        let mut idx = [ia, ib];
        idx.sort_unstable();
        self.project.shapes.remove(idx[1]);
        self.project.shapes.remove(idx[0]);
        let first = self.project.shapes.len();
        let n = result.len();
        self.project.shapes.extend(result);
        self.sel = (first..first + n).collect();
        self.selected = self.sel.last().copied();
        self.status = format!("{} : {} forme(s) produite(s)", op.label(), n);
    }

    /// Export the project to the `io_path` as `.lbrn2`.
    fn export_lbrn2(&mut self) {
        let xml = crate::io::to_lbrn2(&self.project);
        match std::fs::write(&self.io_path, xml) {
            Ok(()) => self.status = format!("Exporté vers {}", self.io_path),
            Err(e) => self.status = format!("Échec export : {e}"),
        }
    }

    /// Import from `io_path`, dispatching on the extension: `.lbrn2` replaces
    /// the project; `.svg` / `.dxf` append their shapes.
    fn import_file(&mut self) {
        let path = self.io_path.clone();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("Lecture impossible : {e}");
                return;
            }
        };
        self.push_undo();
        let ext = std::path::Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "lbrn2" | "lbrn" | "xml" => match crate::io::from_lbrn2(&text) {
                Ok(p) => {
                    self.project = p;
                    self.sel.clear();
                    self.selected = None;
                    self.status = format!("Projet .lbrn2 importé ({} formes)", self.project.shapes.len());
                }
                Err(e) => self.status = format!("Échec import .lbrn2 : {e}"),
            },
            "svg" => self.append_imported(crate::io::import_svg(&text)),
            "dxf" => self.append_imported(crate::io::import_dxf(&text)),
            other => self.status = format!("Extension non gérée : .{other}"),
        }
    }

    fn append_imported(&mut self, result: Result<Vec<Shape>, String>) {
        match result {
            Ok(shapes) => {
                let n = shapes.len();
                self.project.shapes.extend(shapes);
                self.status = format!("{n} forme(s) importée(s)");
            }
            Err(e) => self.status = format!("Échec import : {e}"),
        }
    }

    /// Serialize the project back into the store and persist it.
    fn save(&mut self) {
        let value = match serde_json::to_value(&self.project) {
            Ok(v) => v,
            Err(e) => {
                self.status = format!("Sérialisation impossible : {e}");
                return;
            }
        };
        self.store.set_document(value);
        match self.store.save() {
            Ok(backup) => {
                self.last_backup = backup.clone();
                self.status = match backup {
                    Some(b) => format!(
                        "Enregistré dans {} (sauvegarde : {})",
                        self.store.path().display(),
                        b.display()
                    ),
                    None => format!("Enregistré dans {}", self.store.path().display()),
                };
            }
            Err(e) => self.status = format!("Échec de l'enregistrement : {e}"),
        }
    }

    /// The context toolbar above the canvas: controls that change with the
    /// active tool, the way LightBurn's top toolbar reacts to the current tool.
    fn context_bar(&mut self, ui: &mut egui::Ui) {
        // Text tool (always available): vectorize a string onto the canvas.
        ui.horizontal_wrapped(|ui| {
            ui.strong(self.tool.label());
            ui.separator();
            ui.label("Texte :");
            ui.add(egui::TextEdit::singleline(&mut self.text_input).desired_width(140.0));
            ui.add(egui::DragValue::new(&mut self.text_size).range(2.0..=200.0).speed(0.5).suffix(" mm"));
            if ui.button("Aa Ajouter texte").clicked() {
                self.insert_text();
            }
        });

        // Polygon-specific: number of sides.
        if self.tool == Tool::Polygon {
            ui.horizontal(|ui| {
                ui.label("Côtés :");
                ui.add(egui::DragValue::new(&mut self.poly_sides).range(3..=60).speed(0.1));
                ui.weak("Glisse sur le plan pour tracer le polygone régulier.");
            });
        }

        // Measure-specific: reset + hint.
        if self.tool == Tool::Measure {
            ui.horizontal(|ui| {
                if ui.button("Réinitialiser").clicked() {
                    self.measure_a = None;
                    self.measure_b = None;
                    self.measure_mm = None;
                }
                if let Some(d) = self.measure_mm {
                    ui.strong(format!("Distance : {d:.2} mm"));
                } else {
                    ui.weak("Clique deux points pour mesurer la distance.");
                }
            });
        }

        // Pen-specific controls.
        if self.tool == Tool::Pen {
            ui.horizontal(|ui| {
                let can_finish = self.pending_pen.len() >= 2;
                if ui.add_enabled(can_finish, egui::Button::new("Terminer (ouvert)")).clicked() {
                    self.finish_pen(false);
                }
                if ui
                    .add_enabled(self.pending_pen.len() >= 3, egui::Button::new("Fermer le contour"))
                    .clicked()
                {
                    self.finish_pen(true);
                }
                if ui.add_enabled(!self.pending_pen.is_empty(), egui::Button::new("Annuler")).clicked() {
                    self.pending_pen.clear();
                }
                ui.weak("Clic = ajouter un point · Entrée = terminer · Échap = annuler");
            });
        } else if self.tool == Tool::Node {
            ui.horizontal_wrapped(|ui| {
                let has_node = self.selected_node.is_some() && self.selected_is_path();
                if ui.add_enabled(has_node, egui::Button::new("Courber le segment suivant")).clicked() {
                    self.curve_segment_after_node();
                }
                if ui.add_enabled(has_node, egui::Button::new("Rendre anguleux")).clicked() {
                    self.make_node_corner();
                }
                if ui.add_enabled(has_node, egui::Button::new("Scinder après le nœud")).clicked() {
                    self.split_after_node();
                }
                if ui.add_enabled(has_node, egui::Button::new("🗑 Supprimer le nœud")).clicked() {
                    self.delete_node();
                }
                ui.weak("Glisse les carrés (nœuds) et les ronds (poignées Bézier)");
            });
        } else if self.tool == Tool::Select && !self.sel.is_empty() {
            ui.horizontal_wrapped(|ui| {
                if ui.button("↔ ×1.2").on_hover_text("Agrandir").clicked() {
                    self.scale_selection(1.2);
                }
                if ui.button("↔ ÷1.2").on_hover_text("Réduire").clicked() {
                    self.scale_selection(1.0 / 1.2);
                }
                if ui.button("⟳ +15°").clicked() {
                    self.rotate_selection(15_f64.to_radians());
                }
                if ui.button("⟲ −15°").clicked() {
                    self.rotate_selection(-15_f64.to_radians());
                }
                if ui.button("⇋ Miroir H").clicked() {
                    self.mirror_selection(true);
                }
                if ui.button("⇵ Miroir V").clicked() {
                    self.mirror_selection(false);
                }
                ui.separator();
                if ui.add_enabled(self.sel.len() >= 2, egui::Button::new("Grouper")).clicked() {
                    self.group_selection();
                }
                let is_group = self
                    .selected
                    .and_then(|i| self.project.shapes.get(i))
                    .map(|s| matches!(s.kind, ShapeKind::Group { .. }))
                    .unwrap_or(false);
                if ui.add_enabled(is_group, egui::Button::new("Dégrouper")).clicked() {
                    self.ungroup_selection();
                }
                if ui.button("🗑 Supprimer").clicked() {
                    self.delete_selection();
                }
                ui.weak(format!("{} sélectionnée(s) · Ctrl+clic = ajouter", self.sel.len()));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Tableau :");
                ui.add(egui::DragValue::new(&mut self.array_cols).range(1..=50).prefix("col "));
                ui.add(egui::DragValue::new(&mut self.array_rows).range(1..=50).prefix("lig "));
                ui.add(egui::DragValue::new(&mut self.array_dx).range(0.0..=1000.0).suffix(" mm X"));
                ui.add(egui::DragValue::new(&mut self.array_dy).range(0.0..=1000.0).suffix(" mm Y"));
                if ui.button("Générer le tableau").clicked() {
                    self.array_selection();
                }
                ui.separator();
                let two = self.sel.len() >= 2;
                if ui.add_enabled(two, egui::Button::new("Union")).clicked() {
                    self.boolean_selection(crate::boolean::BoolOp::Union);
                }
                if ui.add_enabled(two, egui::Button::new("Intersection")).clicked() {
                    self.boolean_selection(crate::boolean::BoolOp::Intersection);
                }
                if ui.add_enabled(two, egui::Button::new("Différence")).clicked() {
                    self.boolean_selection(crate::boolean::BoolOp::Difference);
                }
                ui.separator();
                ui.label("Décalage :");
                ui.add(egui::DragValue::new(&mut self.offset_dist).range(-100.0..=100.0).speed(0.1).suffix(" mm"));
                if ui.button("◎ Décaler").on_hover_text("Contour décalé de la forme sélectionnée").clicked() {
                    self.offset_selection(self.offset_dist);
                }
            });
        }
    }

    /// The drawing surface itself: grid, pointer handling, shapes, overlays.
    fn canvas_surface(&mut self, ui: &mut egui::Ui) {
        // --- drawing surface ---
        let avail = ui.available_size();
        let (response, painter) =
            ui.allocate_painter(avail, egui::Sense::click_and_drag());
        let rect = response.rect;
        self.last_canvas = rect;
        painter.rect_filled(rect, 2.0, egui::Color32::from_gray(250));

        let base = rect.left_top() + egui::vec2(40.0, 40.0);

        // Middle-drag pans the view; the wheel zooms about the cursor
        // (like LightBurn's navigation).
        if response.dragged_by(egui::PointerButton::Middle) {
            self.view_offset += response.drag_delta();
        }
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.1 {
            if let Some(cursor) = response.hover_pos() {
                let old_s = self.px_per_mm;
                let new_s = (old_s * (scroll * 0.0015).exp()).clamp(0.5, 20.0);
                let origin = base + self.view_offset;
                let m = (cursor - origin) / old_s;
                self.px_per_mm = new_s;
                self.view_offset = (cursor - base) - m * new_s;
            }
        }

        let origin = base + self.view_offset;
        let s = self.px_per_mm;
        let to_screen = |mx: f64, my: f64| origin + egui::vec2(mx as f32 * s, my as f32 * s);
        let to_model = |p: egui::Pos2| {
            let d = p - origin;
            ((d.x / s) as f64, (d.y / s) as f64)
        };

        draw_grid(&painter, rect, origin, s);

        // --- input handling ---
        let pointer = response.interact_pointer_pos();
        let delete_pressed = ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
        let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let esc_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));

        use egui::PointerButton::Primary;
        match self.tool {
            Tool::Rect | Tool::Ellipse | Tool::Polygon => {
                if response.drag_started_by(Primary) {
                    if let Some(p) = pointer {
                        self.drag_start = Some(to_model(p));
                    }
                }
                if response.drag_stopped_by(Primary) {
                    if let (Some(start), Some(p)) = (self.drag_start.take(), pointer) {
                        let end = to_model(p);
                        self.create_dragged_shape(start, end);
                    }
                }
            }
            Tool::Measure => {
                if response.clicked() {
                    if let Some(p) = pointer {
                        let pt = to_model(p);
                        if self.measure_a.is_none() || self.measure_b.is_some() {
                            // Start (or restart) a measurement.
                            self.measure_a = Some(pt);
                            self.measure_b = None;
                            self.measure_mm = None;
                            self.status = "Mesure : clique le second point".into();
                        } else {
                            let a = self.measure_a.unwrap();
                            let d = ((pt.0 - a.0).powi(2) + (pt.1 - a.1).powi(2)).sqrt();
                            self.measure_b = Some(pt);
                            self.measure_mm = Some(d);
                            self.status = format!("Distance : {d:.2} mm");
                        }
                    }
                }
                if esc_pressed {
                    self.measure_a = None;
                    self.measure_b = None;
                    self.measure_mm = None;
                }
            }
            Tool::Pen => {
                if response.clicked() {
                    if let Some(p) = pointer {
                        self.pending_pen.push(to_model(p));
                    }
                }
                if enter_pressed && self.pending_pen.len() >= 2 {
                    self.finish_pen(false);
                }
                if esc_pressed {
                    self.pending_pen.clear();
                }
            }
            Tool::Select => {
                let ctrl = ui.input(|i| i.modifiers.command);
                if response.clicked() {
                    if let Some(p) = pointer {
                        let hit = self.hit_test(to_model(p));
                        match hit {
                            Some(i) if ctrl => {
                                if let Some(pos) = self.sel.iter().position(|&x| x == i) {
                                    self.sel.remove(pos);
                                } else {
                                    self.sel.push(i);
                                }
                                self.selected = Some(i);
                            }
                            Some(i) => {
                                self.sel = vec![i];
                                self.selected = Some(i);
                            }
                            None => {
                                self.sel.clear();
                                self.selected = None;
                            }
                        }
                    }
                }
                if response.drag_started_by(Primary) {
                    if let Some(p) = pointer {
                        self.moving = self.hit_test(to_model(p)).is_some_and(|i| self.sel.contains(&i));
                        if self.moving {
                            self.push_undo(); // one undo entry per move gesture
                        }
                    }
                }
                if response.dragged_by(Primary) && self.moving {
                    let d = response.drag_delta();
                    self.move_selection(d.x as f64 / s as f64, d.y as f64 / s as f64);
                }
                if response.drag_stopped_by(Primary) {
                    self.moving = false;
                    if self.snap {
                        self.snap_selection();
                    }
                }
                if delete_pressed {
                    self.delete_selection();
                }
            }
            Tool::Node => {
                // Pick a path to edit if none selected yet.
                if response.clicked() && !self.selected_is_path() {
                    if let Some(p) = pointer {
                        self.selected = self.hit_test(to_model(p));
                        self.selected_node = None;
                    }
                }
                if delete_pressed {
                    self.delete_node();
                }
            }
        }

        // --- render committed shapes (flattening groups) ---
        for (i, shape) in self.project.shapes.iter().enumerate() {
            let selected = self.sel.contains(&i) || self.selected == Some(i);
            for (pts, closed, cut_index) in shape.flatten(48, &XForm::IDENTITY) {
                if pts.is_empty() {
                    continue;
                }
                let mut screen: Vec<egui::Pos2> = pts.iter().map(|&(x, y)| to_screen(x, y)).collect();
                if closed {
                    screen.push(screen[0]);
                }
                let color = if selected {
                    egui::Color32::from_rgb(255, 140, 0)
                } else {
                    let (r, g, b) = layer_rgb(cut_index);
                    egui::Color32::from_rgb(r, g, b)
                };
                painter.add(egui::Shape::line(
                    screen,
                    egui::Stroke::new(if selected { 3.0 } else { 1.6 }, color),
                ));
            }
        }

        // --- node editing overlay (draggable vertices + bezier handles) ---
        if self.tool == Tool::Node {
            self.edit_nodes(ui, &painter, origin, s);
        }

        // --- previews ---
        let (r, g, b) = layer_rgb(self.active_layer);
        let preview_color = egui::Color32::from_rgba_unmultiplied(r, g, b, 160);
        if matches!(self.tool, Tool::Rect | Tool::Ellipse | Tool::Polygon) {
            if let (Some(start), Some(p)) = (self.drag_start, response.hover_pos()) {
                let end = to_model(p);
                let preview = self.shape_from_drag(start, end);
                if let Some(shape) = preview {
                    let (pts, closed) = shape.outline(48);
                    let mut screen: Vec<egui::Pos2> =
                        pts.iter().map(|&(x, y)| to_screen(x, y)).collect();
                    if closed && !screen.is_empty() {
                        screen.push(screen[0]);
                    }
                    painter.add(egui::Shape::line(screen, egui::Stroke::new(1.5, preview_color)));
                }
            }
        }
        if self.tool == Tool::Pen && !self.pending_pen.is_empty() {
            let mut screen: Vec<egui::Pos2> =
                self.pending_pen.iter().map(|&(x, y)| to_screen(x, y)).collect();
            if let Some(p) = response.hover_pos() {
                screen.push(p);
            }
            painter.add(egui::Shape::line(screen, egui::Stroke::new(1.5, preview_color)));
            for &(x, y) in &self.pending_pen {
                painter.circle_filled(to_screen(x, y), 3.0, preview_color);
            }
        }

        // --- measure overlay ---
        if self.tool == Tool::Measure {
            let magenta = egui::Color32::from_rgb(200, 0, 160);
            let end = self
                .measure_b
                .or_else(|| response.hover_pos().map(to_model));
            if let (Some(a), Some(b)) = (self.measure_a, end) {
                let (pa, pb) = (to_screen(a.0, a.1), to_screen(b.0, b.1));
                painter.line_segment([pa, pb], egui::Stroke::new(1.5, magenta));
                painter.circle_filled(pa, 3.0, magenta);
                painter.circle_filled(pb, 3.0, magenta);
                let d = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
                painter.text(
                    pb + egui::vec2(8.0, -8.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{d:.2} mm"),
                    egui::FontId::proportional(13.0),
                    magenta,
                );
            } else if let Some(a) = self.measure_a {
                painter.circle_filled(to_screen(a.0, a.1), 3.0, magenta);
            }
        }

        // Footer hint with shape count.
        painter.text(
            rect.left_bottom() + egui::vec2(6.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{} forme(s) · origine (0,0) en haut-gauche, mm", self.project.shapes.len()),
            egui::FontId::proportional(12.0),
            egui::Color32::from_gray(120),
        );
    }

    /// Build a shape from a rubber-band drag, in model coordinates.
    fn shape_from_drag(&self, start: (f64, f64), end: (f64, f64)) -> Option<Shape> {
        let (x0, y0) = start;
        let (x1, y1) = end;
        let (w, h) = ((x1 - x0).abs(), (y1 - y0).abs());
        if w < 0.5 || h < 0.5 {
            return None;
        }
        Some(match self.tool {
            Tool::Rect => Shape::rect(self.active_layer, x0.min(x1), y0.min(y1), w, h),
            Tool::Ellipse => {
                Shape::ellipse(self.active_layer, (x0 + x1) / 2.0, (y0 + y1) / 2.0, w / 2.0, h / 2.0)
            }
            Tool::Polygon => {
                let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                let (rx, ry) = (w / 2.0, h / 2.0);
                let n = self.poly_sides.max(3);
                // Regular polygon inscribed in the drag box, first vertex up.
                let pts: Vec<(f64, f64)> = (0..n)
                    .map(|k| {
                        let a = -std::f64::consts::FRAC_PI_2 + k as f64 * std::f64::consts::TAU / n as f64;
                        (cx + rx * a.cos(), cy + ry * a.sin())
                    })
                    .collect();
                Shape::polyline(self.active_layer, &pts, true)
            }
            _ => return None,
        })
    }

    fn create_dragged_shape(&mut self, start: (f64, f64), end: (f64, f64)) {
        if let Some(shape) = self.shape_from_drag(start, end) {
            self.push_undo();
            self.project.shapes.push(shape);
            self.selected = Some(self.project.shapes.len() - 1);
            self.sel = vec![self.project.shapes.len() - 1];
            self.status = format!("{} ajouté", self.tool.label());
        }
    }

    fn finish_pen(&mut self, closed: bool) {
        if self.pending_pen.len() >= 2 {
            self.push_undo();
            let shape = Shape::polyline(self.active_layer, &self.pending_pen, closed);
            self.project.shapes.push(shape);
            self.selected = Some(self.project.shapes.len() - 1);
            self.sel = vec![self.project.shapes.len() - 1];
            self.status = format!("Tracé de {} points ajouté", self.pending_pen.len());
        }
        self.pending_pen.clear();
    }

    /// Return the topmost shape whose world bounding box contains the point.
    fn hit_test(&self, point: (f64, f64)) -> Option<usize> {
        let (px, py) = point;
        let m = 1.5; // mm tolerance
        for (i, shape) in self.project.shapes.iter().enumerate().rev() {
            if let Some((minx, miny, maxx, maxy)) = shape.world_bbox(48) {
                if px >= minx - m && px <= maxx + m && py >= miny - m && py <= maxy + m {
                    return Some(i);
                }
            }
        }
        None
    }

    // ------------------------------------------------------------------
    // Transforms & grouping (UndoMove/Scale/Rotate/Mirror/GroupShapes):
    // each operation composes a world transform onto the shapes' XForm.
    // ------------------------------------------------------------------

    /// Combined world bounding box of the current multi-selection.
    fn selection_bbox(&self) -> Option<(f64, f64, f64, f64)> {
        let mut bb: Option<(f64, f64, f64, f64)> = None;
        for &i in &self.sel {
            if let Some(s) = self.project.shapes.get(i) {
                if let Some((a, b, c, d)) = s.world_bbox(48) {
                    bb = Some(match bb {
                        None => (a, b, c, d),
                        Some((e, f, g, h)) => (e.min(a), f.min(b), g.max(c), h.max(d)),
                    });
                }
            }
        }
        bb
    }

    /// Apply a world transform to every selected shape.
    fn transform_selection(&mut self, world: XForm) {
        for &i in &self.sel {
            if let Some(s) = self.project.shapes.get_mut(i) {
                s.xform = world.compose(&s.xform);
            }
        }
    }

    fn move_selection(&mut self, dx: f64, dy: f64) {
        self.transform_selection(XForm::translate(dx, dy));
    }

    fn scale_selection(&mut self, factor: f64) {
        if let Some((minx, miny, maxx, maxy)) = self.selection_bbox() {
            self.push_undo();
            let (cx, cy) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
            self.transform_selection(XForm::about(XForm::scale(factor, factor), cx, cy));
            self.status = format!("Échelle ×{factor:.2}");
        }
    }

    fn rotate_selection(&mut self, radians: f64) {
        if let Some((minx, miny, maxx, maxy)) = self.selection_bbox() {
            self.push_undo();
            let (cx, cy) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
            self.transform_selection(XForm::about(XForm::rotate(radians), cx, cy));
            self.status = format!("Rotation {:.0}°", radians.to_degrees());
        }
    }

    fn mirror_selection(&mut self, horizontal: bool) {
        if let Some((minx, miny, maxx, maxy)) = self.selection_bbox() {
            self.push_undo();
            let (cx, cy) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
            let linear = if horizontal { XForm::scale(-1.0, 1.0) } else { XForm::scale(1.0, -1.0) };
            self.transform_selection(XForm::about(linear, cx, cy));
            self.status = "Miroir appliqué".into();
        }
    }

    /// Snap each selected shape's translation to a 1 mm grid.
    fn snap_selection(&mut self) {
        for &i in &self.sel {
            if let Some(s) = self.project.shapes.get_mut(i) {
                s.xform.e = s.xform.e.round();
                s.xform.f = s.xform.f.round();
            }
        }
    }

    fn delete_selection(&mut self) {
        let mut idx: Vec<usize> = self.sel.clone();
        idx.sort_unstable();
        idx.dedup();
        if idx.is_empty() {
            return;
        }
        self.push_undo();
        for &i in idx.iter().rev() {
            if i < self.project.shapes.len() {
                self.project.shapes.remove(i);
            }
        }
        self.sel.clear();
        self.selected = None;
        self.status = format!("{} forme(s) supprimée(s)", idx.len());
    }

    /// Wrap the selected shapes into a single `Group` shape.
    fn group_selection(&mut self) {
        let mut idx: Vec<usize> = self.sel.clone();
        idx.sort_unstable();
        idx.dedup();
        if idx.len() < 2 {
            return;
        }
        self.push_undo();
        let mut children = Vec::new();
        for &i in idx.iter().rev() {
            children.push(self.project.shapes.remove(i));
        }
        children.reverse();
        let cut_index = children.first().map(|s| s.cut_index).unwrap_or(0);
        let group = Shape {
            cut_index,
            xform: XForm::IDENTITY,
            kind: ShapeKind::Group { children },
        };
        self.project.shapes.push(group);
        let gi = self.project.shapes.len() - 1;
        self.sel = vec![gi];
        self.selected = Some(gi);
        self.status = "Formes groupées".into();
    }

    /// Expand the selected group back into its children (baking the group's
    /// XForm into each child).
    fn ungroup_selection(&mut self) {
        let Some(i) = self.selected else { return };
        if i >= self.project.shapes.len() {
            return;
        }
        let group_xform = self.project.shapes[i].xform;
        let ShapeKind::Group { children } = self.project.shapes[i].kind.clone() else {
            return;
        };
        self.push_undo();
        self.project.shapes.remove(i);
        let mut new_indices = Vec::new();
        for mut child in children {
            child.xform = group_xform.compose(&child.xform);
            self.project.shapes.push(child);
            new_indices.push(self.project.shapes.len() - 1);
        }
        self.sel = new_indices;
        self.selected = self.sel.last().copied();
        self.status = "Groupe dissous".into();
    }

    // ------------------------------------------------------------------
    // Node editing (LightBurn's NodeEditTool): draggable vertices and
    // bezier handles, line<->curve, split (UndoSplitPrimitive) and delete
    // (UndoDeleteVertex).
    // ------------------------------------------------------------------

    fn selected_is_path(&self) -> bool {
        matches!(
            self.selected.and_then(|i| self.project.shapes.get(i)).map(|s| &s.kind),
            Some(ShapeKind::Path { .. })
        )
    }

    /// Mutable access to the selected path's (verts, prims), if any.
    fn selected_path_mut(&mut self) -> Option<(&mut Vec<Vertex>, &mut Vec<Prim>)> {
        let i = self.selected?;
        match &mut self.project.shapes.get_mut(i)?.kind {
            ShapeKind::Path { verts, prims, .. } => Some((verts, prims)),
            _ => None,
        }
    }

    fn edit_nodes(&mut self, ui: &egui::Ui, painter: &egui::Painter, origin: egui::Pos2, s: f32) {
        let Some(si) = self.selected else {
            painter.text(
                origin,
                egui::Align2::LEFT_TOP,
                "Outil Nœuds : clique un tracé (plume) pour l'éditer",
                egui::FontId::proportional(13.0),
                egui::Color32::from_gray(120),
            );
            return;
        };
        let Some(shape) = self.project.shapes.get(si) else { return };
        let ShapeKind::Path { .. } = shape.kind else {
            painter.text(
                origin,
                egui::Align2::LEFT_TOP,
                "Sélectionne un tracé (Path) pour éditer ses nœuds",
                egui::FontId::proportional(13.0),
                egui::Color32::from_gray(120),
            );
            return;
        };
        let xform = shape.xform;
        let to_screen = |mx: f64, my: f64| {
            let (wx, wy) = xform.apply(mx, my);
            origin + egui::vec2(wx as f32 * s, wy as f32 * s)
        };
        // Screen drag delta -> local delta (undo the xform's linear part).
        let to_local = move |dx: f32, dy: f32| inverse_linear(&xform, dx as f64 / s as f64, dy as f64 / s as f64);

        let blue = egui::Color32::from_rgb(0, 90, 220);
        let orange = egui::Color32::from_rgb(255, 140, 0);

        // Snapshot geometry for iteration (indices are stable during this frame).
        let (verts, prims): (Vec<Vertex>, Vec<Prim>) = match &self.project.shapes[si].kind {
            ShapeKind::Path { verts, prims, .. } => (verts.clone(), prims.clone()),
            _ => return,
        };

        // Bezier handles: draw connectors and draggable circles.
        for prim in &prims {
            if let Prim::Bezier { a, b } = *prim {
                if let (Some(va), Some(vb)) = (verts.get(a), verts.get(b)) {
                    if let Some(c1) = va.c1 {
                        self.handle_widget(ui, painter, ("c1", si, a), to_screen(c1.0, c1.1), to_screen(va.x, va.y), blue, &to_local, HandleRef::C1(a));
                    }
                    if let Some(c0) = vb.c0 {
                        self.handle_widget(ui, painter, ("c0", si, b), to_screen(c0.0, c0.1), to_screen(vb.x, vb.y), blue, &to_local, HandleRef::C0(b));
                    }
                }
            }
        }

        // Vertices: draggable squares.
        for (vi, v) in verts.iter().enumerate() {
            let sp = to_screen(v.x, v.y);
            let rect = egui::Rect::from_center_size(sp, egui::vec2(9.0, 9.0));
            let id = ui.make_persistent_id(("vtx", si, vi));
            let resp = ui.interact(rect, id, egui::Sense::click_and_drag());
            let sel = self.selected_node == Some(vi);
            painter.rect_filled(rect, 0.0, if sel { orange } else { egui::Color32::WHITE });
            painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.5, blue), egui::StrokeKind::Middle);
            if resp.clicked() {
                self.selected_node = Some(vi);
            }
            if resp.drag_started() {
                self.push_undo();
            }
            if resp.dragged() {
                let d = resp.drag_delta();
                let (dlx, dly) = to_local(d.x, d.y);
                if let Some((vs, _)) = self.selected_path_mut() {
                    if let Some(vt) = vs.get_mut(vi) {
                        vt.x += dlx;
                        vt.y += dly;
                        if let Some(c) = &mut vt.c0 {
                            c.0 += dlx;
                            c.1 += dly;
                        }
                        if let Some(c) = &mut vt.c1 {
                            c.0 += dlx;
                            c.1 += dly;
                        }
                    }
                }
                self.selected_node = Some(vi);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_widget(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        id: (&'static str, usize, usize),
        pos: egui::Pos2,
        anchor: egui::Pos2,
        color: egui::Color32,
        to_local: &dyn Fn(f32, f32) -> (f64, f64),
        href: HandleRef,
    ) {
        painter.line_segment([anchor, pos], egui::Stroke::new(1.0, color));
        let rect = egui::Rect::from_center_size(pos, egui::vec2(9.0, 9.0));
        let resp = ui.interact(rect, ui.make_persistent_id(id), egui::Sense::drag());
        painter.circle_filled(pos, 4.0, color);
        if resp.drag_started() {
            self.push_undo();
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            let (dlx, dly) = to_local(d.x, d.y);
            if let Some((vs, _)) = self.selected_path_mut() {
                let (vi, slot) = match href {
                    HandleRef::C0(i) => (i, 0),
                    HandleRef::C1(i) => (i, 1),
                };
                if let Some(vt) = vs.get_mut(vi) {
                    let h = if slot == 0 { &mut vt.c0 } else { &mut vt.c1 };
                    if let Some(c) = h {
                        c.0 += dlx;
                        c.1 += dly;
                    }
                }
            }
        }
    }

    /// Convert the Line primitive leaving the selected node into a Bezier,
    /// seeding the handles at the thirds of the segment.
    fn curve_segment_after_node(&mut self) {
        let Some(n) = self.selected_node else { return };
        if !self.selected_is_path() {
            return;
        }
        self.push_undo();
        let Some((verts, prims)) = self.selected_path_mut() else { return };
        let Some(pi) = prims.iter().position(|p| matches!(p, Prim::Line { a, .. } if *a == n)) else {
            return;
        };
        let (a, b) = match prims[pi] {
            Prim::Line { a, b } => (a, b),
            _ => return,
        };
        let (Some(va), Some(vb)) = (verts.get(a).copied(), verts.get(b).copied()) else { return };
        let dx = vb.x - va.x;
        let dy = vb.y - va.y;
        verts[a].c1 = Some((va.x + dx / 3.0, va.y + dy / 3.0));
        verts[b].c0 = Some((vb.x - dx / 3.0, vb.y - dy / 3.0));
        prims[pi] = Prim::Bezier { a, b };
        self.status = "Segment converti en courbe de Bézier".into();
    }

    /// Make the selected node a sharp corner: drop its handles and turn the
    /// adjacent bezier primitives back into lines.
    fn make_node_corner(&mut self) {
        let Some(n) = self.selected_node else { return };
        if !self.selected_is_path() {
            return;
        }
        self.push_undo();
        let Some((verts, prims)) = self.selected_path_mut() else { return };
        if let Some(v) = verts.get_mut(n) {
            v.c0 = None;
            v.c1 = None;
        }
        for p in prims.iter_mut() {
            if let Prim::Bezier { a, b } = *p {
                if a == n || b == n {
                    *p = Prim::Line { a, b };
                }
            }
        }
        self.status = "Nœud rendu anguleux".into();
    }

    /// Split the primitive leaving the selected node at its midpoint,
    /// inserting a new vertex (LightBurn's UndoSplitPrimitive).
    fn split_after_node(&mut self) {
        let Some(n) = self.selected_node else { return };
        if !self.selected_is_path() {
            return;
        }
        self.push_undo();
        let Some((verts, prims)) = self.selected_path_mut() else { return };
        let Some(pi) = prims.iter().position(|p| match p {
            Prim::Line { a, .. } | Prim::Bezier { a, .. } => *a == n,
        }) else {
            return;
        };
        match prims[pi] {
            Prim::Line { a, b } => {
                let (Some(va), Some(vb)) = (verts.get(a).copied(), verts.get(b).copied()) else {
                    return;
                };
                let mid = Vertex::corner((va.x + vb.x) / 2.0, (va.y + vb.y) / 2.0);
                let m = verts.len();
                verts.push(mid);
                prims[pi] = Prim::Line { a, b: m };
                prims.insert(pi + 1, Prim::Line { a: m, b });
            }
            Prim::Bezier { a, b } => {
                let (Some(va), Some(vb)) = (verts.get(a).copied(), verts.get(b).copied()) else {
                    return;
                };
                let p0 = (va.x, va.y);
                let p1 = va.c1.unwrap_or(p0);
                let p3 = (vb.x, vb.y);
                let p2 = vb.c0.unwrap_or(p3);
                let (left, mid, right) = split_cubic(p0, p1, p2, p3, 0.5);
                let m = verts.len();
                // New middle vertex with handles toward both sides.
                let mut mv = Vertex::corner(mid.0, mid.1);
                mv.c0 = Some(left.2); // incoming handle (= d)
                mv.c1 = Some(right.0); // outgoing handle (= e)
                verts.push(mv);
                verts[a].c1 = Some(left.1); // start's outgoing handle (= a)
                verts[b].c0 = Some(right.1); // end's incoming handle (= c)
                prims[pi] = Prim::Bezier { a, b: m };
                prims.insert(pi + 1, Prim::Bezier { a: m, b });
            }
        }
        self.status = "Primitive scindée".into();
    }

    /// Delete the selected vertex, dropping incident primitives and
    /// reconnecting its neighbours with a line (LightBurn's UndoDeleteVertex).
    fn delete_node(&mut self) {
        let Some(n) = self.selected_node else { return };
        if !self.selected_is_path() {
            return;
        }
        self.push_undo();
        self.selected_node = None;
        let Some((verts, prims)) = self.selected_path_mut() else { return };
        if verts.len() <= 2 {
            return;
        }
        // Find neighbours through incident primitives.
        let mut prev = None;
        let mut next = None;
        for p in prims.iter() {
            match *p {
                Prim::Line { a, b } | Prim::Bezier { a, b } => {
                    if b == n {
                        prev = Some(a);
                    }
                    if a == n {
                        next = Some(b);
                    }
                }
            }
        }
        prims.retain(|p| !matches!(p, Prim::Line { a, b } | Prim::Bezier { a, b } if *a == n || *b == n));
        if let (Some(a), Some(b)) = (prev, next) {
            prims.push(Prim::Line { a, b });
        }
        verts.remove(n);
        // Shift indices above the removed vertex.
        for p in prims.iter_mut() {
            match p {
                Prim::Line { a, b } | Prim::Bezier { a, b } => {
                    if *a > n {
                        *a -= 1;
                    }
                    if *b > n {
                        *b -= 1;
                    }
                }
            }
        }
        self.status = "Nœud supprimé".into();
    }

    fn layers_tab(&mut self, ui: &mut egui::Ui) {
        if ui.button("➕ Ajouter un calque").clicked() {
            // Next free palette slot (indices stay stable so shapes keep their CutIndex).
            let idx = self.project.layers.iter().map(|l| l.index).max().map(|m| m + 1).unwrap_or(0);
            self.project.layers.push(CutLayer::new(idx));
        }
        ui.weak("L'ordre de la liste = ordre de découpe (priorité). Utilise ▲▼ pour le changer.");
        ui.separator();

        let mut remove: Option<usize> = None;
        let mut move_up: Option<usize> = None;
        let mut move_down: Option<usize> = None;
        let count = self.project.layers.len();
        egui::Grid::new("layers_grid")
            .striped(true)
            .num_columns(9)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                for h in ["Ordre", "Couleur", "Nom", "Type", "Vitesse", "Puiss. max", "Puiss. min", "Passes", ""] {
                    ui.strong(h);
                }
                ui.end_row();

                for i in 0..count {
                    let layer = &mut self.project.layers[i];
                    ui.label(format!("{}  (slot {})", i + 1, layer.index));

                    let (r, g, b) = layer.rgb();
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 14.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(r, g, b));

                    ui.text_edit_singleline(&mut layer.name);

                    egui::ComboBox::from_id_salt(("layer_kind", i))
                        .selected_text(layer.kind.label())
                        .show_ui(ui, |ui| {
                            for k in LayerType::ALL {
                                ui.selectable_value(&mut layer.kind, k, k.label());
                            }
                        });

                    ui.add(egui::DragValue::new(&mut layer.speed).range(0.0..=10_000.0).speed(1.0).suffix(" mm/s"));
                    ui.add(egui::DragValue::new(&mut layer.max_power).range(0.0..=100.0).speed(0.5).suffix(" %"));
                    ui.add(egui::DragValue::new(&mut layer.min_power).range(0.0..=100.0).speed(0.5).suffix(" %"));
                    ui.add(egui::DragValue::new(&mut layer.passes).range(1..=50).speed(0.1));

                    ui.horizontal(|ui| {
                        if ui.add_enabled(i > 0, egui::Button::new("▲")).clicked() {
                            move_up = Some(i);
                        }
                        if ui.add_enabled(i + 1 < count, egui::Button::new("▼")).clicked() {
                            move_down = Some(i);
                        }
                        if ui.button("🗑").clicked() {
                            remove = Some(i);
                        }
                    });
                    ui.end_row();
                }
            });

        // Reorder = change cut priority; indices stay stable (shapes keep refs).
        if let Some(i) = move_up {
            self.project.layers.swap(i, i - 1);
        }
        if let Some(i) = move_down {
            self.project.layers.swap(i, i + 1);
        }
        if let Some(i) = remove {
            self.project.layers.remove(i);
        }
    }

    fn devices_tab(&mut self, ui: &mut egui::Ui) {
        if ui.button("➕ Ajouter une machine").clicked() {
            self.project.devices.push(Device::new("Nouvelle machine"));
        }
        ui.separator();

        let mut remove: Option<usize> = None;
        egui::Grid::new("devices_grid")
            .striped(true)
            .num_columns(5)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.strong("Nom");
                ui.strong("Largeur (mm)");
                ui.strong("Hauteur (mm)");
                ui.strong("Origine");
                ui.strong("");
                ui.end_row();

                for i in 0..self.project.devices.len() {
                    let dev = &mut self.project.devices[i];
                    ui.text_edit_singleline(&mut dev.name);
                    ui.add(egui::DragValue::new(&mut dev.width_mm).range(1.0..=5_000.0).speed(1.0));
                    ui.add(egui::DragValue::new(&mut dev.height_mm).range(1.0..=5_000.0).speed(1.0));

                    egui::ComboBox::from_id_salt(("dev_origin", i))
                        .selected_text(dev.origin.label())
                        .show_ui(ui, |ui| {
                            for o in Origin::ALL {
                                ui.selectable_value(&mut dev.origin, o, o.label());
                            }
                        });

                    if ui.button("🗑").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });

        if let Some(i) = remove {
            self.project.devices.remove(i);
        }
    }

    fn gcode_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Avance rapide :");
            ui.add(egui::DragValue::new(&mut self.gopts.travel_feed).range(100.0..=30000.0).speed(50.0).suffix(" mm/min"));
            ui.separator();
            ui.label("S à 100% :");
            ui.add(egui::DragValue::new(&mut self.gopts.power_scale).range(1.0..=10000.0).speed(1.0));
            ui.separator();
            ui.checkbox(&mut self.gopts.dynamic_power, "Puissance dynamique (M4)");
        });
        ui.horizontal(|ui| {
            if ui.button("⚙ Générer le G-code").clicked() {
                let job = crate::gcode::generate(&self.project, &self.gopts);
                self.status = format!(
                    "G-code généré : {:.1} mm de découpe, ~{}",
                    job.cut_mm,
                    crate::gcode::fmt_duration(job.seconds)
                );
                self.job = Some(job);
            }
            if let Some(job) = &self.job {
                if ui.button("💾 Enregistrer .gcode").clicked() {
                    let path = std::path::Path::new(&self.io_path).with_extension("gcode");
                    match std::fs::write(&path, &job.gcode) {
                        Ok(()) => self.status = format!("G-code enregistré : {}", path.display()),
                        Err(e) => self.status = format!("Échec : {e}"),
                    }
                }
            }
        });
        ui.separator();

        if let Some(job) = &self.job {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("⏱ Durée estimée : {}", crate::gcode::fmt_duration(job.seconds))).strong());
                ui.separator();
                ui.label(format!("Découpe : {:.1} mm", job.cut_mm));
                ui.separator();
                ui.label(format!("Déplacement : {:.1} mm", job.travel_mm));
                ui.separator();
                ui.label(format!("{} lignes", job.gcode.lines().count()));
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                let mut code = job.gcode.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut code)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
            });
        } else {
            ui.weak("Clique « Générer le G-code » pour produire le programme GRBL et l'estimation de durée.");
        }
    }

    fn prefs_tab(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("prefs_grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            ui.label("Unités");
            egui::ComboBox::from_id_salt("units")
                .selected_text(self.project.units.label())
                .show_ui(ui, |ui| {
                    for u in Units::ALL {
                        ui.selectable_value(&mut self.project.units, u, u.label());
                    }
                });
            ui.end_row();

            ui.label("Sauvegarde auto (s)");
            ui.add(egui::DragValue::new(&mut self.project.auto_save_seconds).range(0..=3600).speed(1.0));
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.label(
            "Ces préférences sont stockées comme dans LightBurn : clés `units` \
             et `AutoSaveSeconds` dans le document JSON.",
        );
    }

    // ==================================================================
    //  LightBurn-style window chrome: menu bar, toolbars, tool palette,
    //  Cuts/Layers dock, canvas and layer-colour strip.
    // ==================================================================

    /// Scale the current selection about an arbitrary centre.
    fn scale_about(&mut self, fx: f64, fy: f64, cx: f64, cy: f64) {
        self.transform_selection(XForm::about(XForm::scale(fx, fy), cx, cy));
    }

    /// Make `slot` the active cut layer; create it if missing, and — as
    /// LightBurn does when a colour is clicked — move any selected shapes onto it.
    fn set_active_slot(&mut self, slot: u32) {
        self.active_layer = slot;
        if !self.project.layers.iter().any(|l| l.index == slot) {
            self.project.layers.push(CutLayer::new(slot));
        }
        if !self.sel.is_empty() {
            for &i in &self.sel {
                if let Some(s) = self.project.shapes.get_mut(i) {
                    s.cut_index = slot;
                }
            }
            self.status = format!("Sélection déplacée sur le calque {slot:02}");
        } else {
            self.status = format!("Calque actif : {slot:02}");
        }
    }

    // ------------------------------------------------------------------
    //  Undo / redo history (LightBurn keeps a full edit stack). Each
    //  discrete action snapshots the whole project before mutating it.
    // ------------------------------------------------------------------

    /// Snapshot the project before a mutating action, for undo.
    fn push_undo(&mut self) {
        self.redo_stack.clear();
        self.undo_stack.push(self.project.clone());
        if self.undo_stack.len() > 80 {
            let excess = self.undo_stack.len() - 80;
            self.undo_stack.drain(0..excess);
        }
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(std::mem::replace(&mut self.project, prev));
            self.clear_selection();
            self.status = "Annuler".into();
        } else {
            self.status = "Rien à annuler".into();
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(std::mem::replace(&mut self.project, next));
            self.clear_selection();
            self.status = "Rétablir".into();
        } else {
            self.status = "Rien à rétablir".into();
        }
    }

    fn clear_selection(&mut self) {
        self.sel.clear();
        self.selected = None;
        self.selected_node = None;
    }

    // ------------------------------------------------------------------
    //  Clipboard: cut / copy / paste / duplicate (LightBurn's Edit menu).
    // ------------------------------------------------------------------

    /// Shapes currently selected, cloned in source order.
    fn selected_shapes(&self) -> Vec<Shape> {
        let mut idx: Vec<usize> = self.sel.clone();
        idx.sort_unstable();
        idx.dedup();
        idx.iter().filter_map(|&i| self.project.shapes.get(i).cloned()).collect()
    }

    fn copy_selection(&mut self) {
        let shapes = self.selected_shapes();
        if !shapes.is_empty() {
            let n = shapes.len();
            self.clipboard = shapes;
            self.status = format!("{n} forme(s) copiée(s)");
        }
    }

    fn cut_selection(&mut self) {
        let shapes = self.selected_shapes();
        if shapes.is_empty() {
            return;
        }
        self.push_undo();
        self.clipboard = shapes;
        let n = self.clipboard.len();
        self.delete_selection_no_undo();
        self.status = format!("{n} forme(s) coupée(s)");
    }

    /// Paste the clipboard, offset by 5 mm so the copies are visible, and
    /// select the result.
    fn paste(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        self.push_undo();
        let off = XForm::translate(5.0, 5.0);
        let first = self.project.shapes.len();
        for s in &self.clipboard {
            let mut clone = s.clone();
            clone.xform = off.compose(&clone.xform);
            self.project.shapes.push(clone);
        }
        let n = self.clipboard.len();
        self.sel = (first..first + n).collect();
        self.selected = self.sel.last().copied();
        self.status = format!("{n} forme(s) collée(s)");
    }

    /// Copy + paste in one step (LightBurn's Duplicate).
    fn duplicate_selection(&mut self) {
        let shapes = self.selected_shapes();
        if shapes.is_empty() {
            return;
        }
        self.push_undo();
        let off = XForm::translate(5.0, 5.0);
        let first = self.project.shapes.len();
        for s in &shapes {
            let mut clone = s.clone();
            clone.xform = off.compose(&clone.xform);
            self.project.shapes.push(clone);
        }
        let n = shapes.len();
        self.sel = (first..first + n).collect();
        self.selected = self.sel.last().copied();
        self.status = format!("{n} forme(s) dupliquée(s)");
    }

    fn select_all(&mut self) {
        self.sel = (0..self.project.shapes.len()).collect();
        self.selected = self.sel.last().copied();
        self.status = format!("{} forme(s) sélectionnée(s)", self.sel.len());
    }

    /// Delete selection without pushing an undo entry (callers that already did).
    fn delete_selection_no_undo(&mut self) {
        let mut idx: Vec<usize> = self.sel.clone();
        idx.sort_unstable();
        idx.dedup();
        for &i in idx.iter().rev() {
            if i < self.project.shapes.len() {
                self.project.shapes.remove(i);
            }
        }
        self.clear_selection();
    }

    /// Start a new, empty project (keeps layers/devices/prefs).
    fn new_project(&mut self) {
        self.push_undo();
        self.project.shapes.clear();
        self.clear_selection();
        self.status = "Nouveau projet".into();
    }

    // ------------------------------------------------------------------
    //  Alignment & 90° rotation (LightBurn's Arrange toolbar).
    // ------------------------------------------------------------------

    /// Align every selected shape within the selection's bounding box.
    fn align_selection(&mut self, align: Align) {
        if self.sel.len() < 2 {
            return;
        }
        let Some((sminx, sminy, smaxx, smaxy)) = self.selection_bbox() else { return };
        self.push_undo();
        let idx: Vec<usize> = self.sel.clone();
        for i in idx {
            let Some(shape) = self.project.shapes.get(i) else { continue };
            let Some((minx, miny, maxx, maxy)) = shape.world_bbox(48) else { continue };
            let (dx, dy) = match align {
                Align::Left => (sminx - minx, 0.0),
                Align::Right => (smaxx - maxx, 0.0),
                Align::Top => (0.0, sminy - miny),
                Align::Bottom => (0.0, smaxy - maxy),
                Align::CenterH => ((sminx + smaxx) / 2.0 - (minx + maxx) / 2.0, 0.0),
                Align::CenterV => (0.0, (sminy + smaxy) / 2.0 - (miny + maxy) / 2.0),
            };
            if let Some(s) = self.project.shapes.get_mut(i) {
                s.xform = XForm::translate(dx, dy).compose(&s.xform);
            }
        }
        self.status = format!("Alignement : {}", align.label());
    }

    fn rotate_90(&mut self, clockwise: bool) {
        // rotate_selection pushes its own undo snapshot.
        let a = if clockwise { 90_f64 } else { -90_f64 };
        self.rotate_selection(a.to_radians());
    }

    // ------------------------------------------------------------------
    //  Offset contour (LightBurn's "Offset Shapes"): grow or shrink the
    //  selected closed shape's outline by a distance, along vertex normals.
    // ------------------------------------------------------------------

    fn offset_selection(&mut self, dist: f64) {
        let Some(&i) = self.sel.first() else {
            self.status = "Sélectionne une forme fermée à décaler".into();
            return;
        };
        let Some(shape) = self.project.shapes.get(i) else { return };
        let flat = shape.flatten(96, &XForm::IDENTITY);
        let Some((pts, closed, cut)) = flat.into_iter().find(|(p, c, _)| *c && p.len() >= 3) else {
            self.status = "Décalage : forme fermée requise".into();
            return;
        };
        let offset = offset_ring(&pts, dist, closed);
        if offset.len() < 3 {
            self.status = "Décalage impossible (distance trop grande ?)".into();
            return;
        }
        self.push_undo();
        self.project.shapes.push(Shape::polyline(cut, &offset, true));
        let ni = self.project.shapes.len() - 1;
        self.sel = vec![ni];
        self.selected = Some(ni);
        self.status = format!("Contour décalé de {dist:+.1} mm");
    }

    // ------------------------------------------------------------------
    //  View: zoom-to-fit / zoom-to-selection / reset (LightBurn's View bar).
    // ------------------------------------------------------------------

    /// Fit `bbox` (model mm) inside the last canvas rectangle.
    fn fit_bbox(&mut self, bbox: Option<(f64, f64, f64, f64)>) {
        let Some((minx, miny, maxx, maxy)) = bbox else { return };
        let (w, h) = ((maxx - minx).max(1.0), (maxy - miny).max(1.0));
        let avail = self.last_canvas.size() - egui::vec2(80.0, 80.0);
        let sx = avail.x / w as f32;
        let sy = avail.y / h as f32;
        let s = sx.min(sy).clamp(0.5, 20.0);
        self.px_per_mm = s;
        // Place the bbox's top-left at (40,40) from the canvas corner.
        self.view_offset = egui::vec2(-(minx as f32) * s, -(miny as f32) * s);
    }

    fn zoom_fit(&mut self) {
        let bbox = self.project_bbox();
        self.fit_bbox(bbox);
        self.status = "Ajusté à la page".into();
    }

    fn zoom_selection(&mut self) {
        if self.sel.is_empty() {
            self.zoom_fit();
            return;
        }
        let bbox = self.selection_bbox();
        self.fit_bbox(bbox);
        self.status = "Zoom sur la sélection".into();
    }

    fn reset_view(&mut self) {
        self.px_per_mm = 2.0;
        self.view_offset = egui::Vec2::ZERO;
        self.status = "Vue réinitialisée".into();
    }

    // ------------------------------------------------------------------
    //  Image trace (the "camera" tool): raster → vector contours.
    // ------------------------------------------------------------------

    /// Vectorize the configured image and drop the contours onto the canvas.
    fn run_trace(&mut self) {
        if self.trace_path.trim().is_empty() {
            self.status = "Indique un fichier image (.png / .jpg)".into();
            return;
        }
        match crate::trace::trace_file(self.trace_path.trim(), &self.trace_opts) {
            Ok(loops) => {
                if loops.is_empty() {
                    self.status = "Aucun contour trouvé — ajuste le seuil ou coche « inverser »".into();
                    return;
                }
                self.push_undo();
                let children: Vec<Shape> = loops
                    .iter()
                    .map(|lp| Shape::polyline(self.active_layer, lp, true))
                    .collect();
                let n = children.len();
                let group = Shape {
                    cut_index: self.active_layer,
                    xform: XForm::IDENTITY,
                    kind: ShapeKind::Group { children },
                };
                self.project.shapes.push(group);
                let gi = self.project.shapes.len() - 1;
                self.sel = vec![gi];
                self.selected = Some(gi);
                self.status = format!("Image tracée : {n} contour(s) vectorisé(s)");
            }
            Err(e) => self.status = e,
        }
    }

    /// The "Camera / trace image" window (raster → cut paths).
    fn camera_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "Vectorise une image (PNG / JPEG) en tracés de découpe — \
             l'équivalent du « tracer l'image » de LightBurn.",
        );
        ui.separator();
        egui::Grid::new("trace_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label("Fichier image");
            ui.add(
                egui::TextEdit::singleline(&mut self.trace_path)
                    .desired_width(320.0)
                    .hint_text("/chemin/vers/image.png"),
            );
            ui.end_row();
            ui.label("Seuil (encre si plus sombre)");
            ui.add(egui::Slider::new(&mut self.trace_opts.threshold, 0..=255));
            ui.end_row();
            ui.label("Inverser");
            ui.checkbox(&mut self.trace_opts.invert, "pixels clairs = encre");
            ui.end_row();
            ui.label("Largeur cible");
            ui.add(egui::DragValue::new(&mut self.trace_opts.target_width_mm).range(5.0..=2000.0).speed(1.0).suffix(" mm"));
            ui.end_row();
            ui.label("Simplification");
            ui.add(egui::DragValue::new(&mut self.trace_opts.simplify_mm).range(0.0..=5.0).speed(0.05).suffix(" mm"));
            ui.end_row();
        });
        ui.separator();
        if ui.button("📷 Tracer et ajouter au projet").clicked() {
            self.run_trace();
        }
        ui.add_space(4.0);
        ui.weak("Astuce : baisse le seuil s'il y a trop de contours, monte-le s'il en manque.");
        ui.add_space(4.0);
        ui.label(&self.status);
    }

    /// The top menu bar (File / Edit / Arrange / Laser Tools / Window / Help).
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.menu_button("Fichier", |ui| {
                if ui.button("🆕 Nouveau projet").clicked() {
                    self.new_project();
                    ui.close();
                }
                if ui.button("💾 Enregistrer").clicked() {
                    self.save();
                    ui.close();
                }
                ui.separator();
                if ui.button("📤 Exporter .lbrn2").clicked() {
                    self.export_lbrn2();
                    ui.close();
                }
                if ui.button("📥 Importer (.lbrn2 / .svg / .dxf)").clicked() {
                    self.import_file();
                    ui.close();
                }
            });
            ui.menu_button("Édition", |ui| {
                if ui.button("↶ Annuler").clicked() {
                    self.undo();
                    ui.close();
                }
                if ui.button("↷ Rétablir").clicked() {
                    self.redo();
                    ui.close();
                }
                ui.separator();
                if ui.button("✂ Couper").clicked() {
                    self.cut_selection();
                    ui.close();
                }
                if ui.button("⎘ Copier").clicked() {
                    self.copy_selection();
                    ui.close();
                }
                if ui.button("📋 Coller").clicked() {
                    self.paste();
                    ui.close();
                }
                if ui.button("⧉ Dupliquer").clicked() {
                    self.duplicate_selection();
                    ui.close();
                }
                ui.separator();
                if ui.button("Tout sélectionner").clicked() {
                    self.select_all();
                    ui.close();
                }
                if ui.button("Tout désélectionner").clicked() {
                    self.clear_selection();
                    ui.close();
                }
                if ui.button("🗑 Supprimer la sélection").clicked() {
                    self.delete_selection();
                    ui.close();
                }
            });
            ui.menu_button("Organiser", |ui| {
                if ui.button("Grouper").clicked() {
                    self.group_selection();
                    ui.close();
                }
                if ui.button("Dégrouper").clicked() {
                    self.ungroup_selection();
                    ui.close();
                }
                ui.separator();
                if ui.button("Miroir horizontal").clicked() {
                    self.mirror_selection(true);
                    ui.close();
                }
                if ui.button("Miroir vertical").clicked() {
                    self.mirror_selection(false);
                    ui.close();
                }
                ui.separator();
                if ui.button("Rotation +90°").clicked() {
                    self.rotate_90(true);
                    ui.close();
                }
                if ui.button("Rotation −90°").clicked() {
                    self.rotate_90(false);
                    ui.close();
                }
                if ui.button("Rotation +15°").clicked() {
                    self.rotate_selection(15_f64.to_radians());
                    ui.close();
                }
                if ui.button("Agrandir ×1.2").clicked() {
                    self.scale_selection(1.2);
                    ui.close();
                }
                if ui.button("Réduire ÷1.2").clicked() {
                    self.scale_selection(1.0 / 1.2);
                    ui.close();
                }
                ui.separator();
                if ui.button("Aligner à gauche").clicked() {
                    self.align_selection(Align::Left);
                    ui.close();
                }
                if ui.button("Aligner à droite").clicked() {
                    self.align_selection(Align::Right);
                    ui.close();
                }
                if ui.button("Aligner en haut").clicked() {
                    self.align_selection(Align::Top);
                    ui.close();
                }
                if ui.button("Aligner en bas").clicked() {
                    self.align_selection(Align::Bottom);
                    ui.close();
                }
                if ui.button("Centrer horizontalement").clicked() {
                    self.align_selection(Align::CenterH);
                    ui.close();
                }
                if ui.button("Centrer verticalement").clicked() {
                    self.align_selection(Align::CenterV);
                    ui.close();
                }
                ui.separator();
                if ui.button("◎ Décaler le contour…").clicked() {
                    self.offset_selection(self.offset_dist);
                    ui.close();
                }
            });
            ui.menu_button("Affichage", |ui| {
                if ui.button("⤢ Ajuster à la page").clicked() {
                    self.zoom_fit();
                    ui.close();
                }
                if ui.button("🔍 Zoom sur la sélection").clicked() {
                    self.zoom_selection();
                    ui.close();
                }
                if ui.button("Réinitialiser la vue").clicked() {
                    self.reset_view();
                    ui.close();
                }
            });
            ui.menu_button("Outils laser", |ui| {
                if ui.button("📷 Tracer une image…").clicked() {
                    self.windows.camera = true;
                    ui.close();
                }
                if ui.button("⚙ Générer le G-code…").clicked() {
                    self.windows.gcode = true;
                    ui.close();
                }
                if ui.button("🔌 Console laser (GRBL)…").clicked() {
                    self.windows.machine = true;
                    ui.close();
                }
            });
            ui.menu_button("Fenêtre", |ui| {
                if ui.button("Réglages des calques…").clicked() {
                    self.windows.cut_settings = true;
                    ui.close();
                }
                if ui.button("Machines…").clicked() {
                    self.windows.devices = true;
                    ui.close();
                }
                if ui.button("Préférences…").clicked() {
                    self.windows.prefs = true;
                    ui.close();
                }
            });
            ui.menu_button("Langue", |ui| {
                ui.label("Français (actif)");
            });
            ui.menu_button("Aide", |ui| {
                if ui.button("À propos de BrûleLumière").clicked() {
                    self.windows.about = true;
                    ui.close();
                }
            });
        });
    }

    /// The icon toolbar under the menu (file · history · clipboard · arrange ·
    /// align · view), each icon wired to its action — like LightBurn's toolbars.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let has_sel = !self.sel.is_empty();
        let two = self.sel.len() >= 2;
        ui.horizontal_wrapped(|ui| {
            // File
            if ui.button("🆕").on_hover_text("Nouveau projet").clicked() {
                self.new_project();
            }
            if ui.button("💾").on_hover_text("Enregistrer (Ctrl+S)").clicked() {
                self.save();
            }
            if ui.button("📥").on_hover_text("Importer (.lbrn2/.svg/.dxf)").clicked() {
                self.import_file();
            }
            if ui.button("📤").on_hover_text("Exporter .lbrn2").clicked() {
                self.export_lbrn2();
            }
            ui.separator();
            // History
            if ui.add_enabled(!self.undo_stack.is_empty(), egui::Button::new("↶")).on_hover_text("Annuler (Ctrl+Z)").clicked() {
                self.undo();
            }
            if ui.add_enabled(!self.redo_stack.is_empty(), egui::Button::new("↷")).on_hover_text("Rétablir (Ctrl+Y)").clicked() {
                self.redo();
            }
            ui.separator();
            // Clipboard
            if ui.add_enabled(has_sel, egui::Button::new("✂")).on_hover_text("Couper (Ctrl+X)").clicked() {
                self.cut_selection();
            }
            if ui.add_enabled(has_sel, egui::Button::new("⎘")).on_hover_text("Copier (Ctrl+C)").clicked() {
                self.copy_selection();
            }
            if ui.add_enabled(!self.clipboard.is_empty(), egui::Button::new("📋")).on_hover_text("Coller (Ctrl+V)").clicked() {
                self.paste();
            }
            if ui.add_enabled(has_sel, egui::Button::new("⧉")).on_hover_text("Dupliquer (Ctrl+D)").clicked() {
                self.duplicate_selection();
            }
            if ui.add_enabled(has_sel, egui::Button::new("🗑")).on_hover_text("Supprimer (Suppr)").clicked() {
                self.delete_selection();
            }
            ui.separator();
            // Arrange
            if ui.add_enabled(two, egui::Button::new("⊞")).on_hover_text("Grouper").clicked() {
                self.group_selection();
            }
            if ui.add_enabled(has_sel, egui::Button::new("⊟")).on_hover_text("Dégrouper").clicked() {
                self.ungroup_selection();
            }
            if ui.add_enabled(has_sel, egui::Button::new("⇋")).on_hover_text("Miroir horizontal").clicked() {
                self.mirror_selection(true);
            }
            if ui.add_enabled(has_sel, egui::Button::new("⇵")).on_hover_text("Miroir vertical").clicked() {
                self.mirror_selection(false);
            }
            if ui.add_enabled(has_sel, egui::Button::new("⟳")).on_hover_text("Rotation +90°").clicked() {
                self.rotate_90(true);
            }
            if ui.add_enabled(has_sel, egui::Button::new("⟲")).on_hover_text("Rotation −90°").clicked() {
                self.rotate_90(false);
            }
            ui.separator();
            // Align
            if ui.add_enabled(two, egui::Button::new("⇤")).on_hover_text("Aligner à gauche").clicked() {
                self.align_selection(Align::Left);
            }
            if ui.add_enabled(two, egui::Button::new("⇥")).on_hover_text("Aligner à droite").clicked() {
                self.align_selection(Align::Right);
            }
            if ui.add_enabled(two, egui::Button::new("⤒")).on_hover_text("Aligner en haut").clicked() {
                self.align_selection(Align::Top);
            }
            if ui.add_enabled(two, egui::Button::new("⤓")).on_hover_text("Aligner en bas").clicked() {
                self.align_selection(Align::Bottom);
            }
            if ui.add_enabled(two, egui::Button::new("↔")).on_hover_text("Centrer horizontalement").clicked() {
                self.align_selection(Align::CenterH);
            }
            if ui.add_enabled(two, egui::Button::new("↕")).on_hover_text("Centrer verticalement").clicked() {
                self.align_selection(Align::CenterV);
            }
            ui.separator();
            // View
            if ui.button("➕").on_hover_text("Zoom avant").clicked() {
                self.px_per_mm = (self.px_per_mm * 1.25).min(20.0);
            }
            if ui.button("➖").on_hover_text("Zoom arrière").clicked() {
                self.px_per_mm = (self.px_per_mm / 1.25).max(0.5);
            }
            if ui.button("⤢").on_hover_text("Ajuster à la page").clicked() {
                self.zoom_fit();
            }
            if ui.add_enabled(has_sel, egui::Button::new("🔍")).on_hover_text("Zoom sur la sélection").clicked() {
                self.zoom_selection();
            }
            if ui.button("⟳0").on_hover_text("Réinitialiser la vue").clicked() {
                self.reset_view();
            }
            ui.separator();
            if ui.button("📷").on_hover_text("Tracer une image (vectorisation)").clicked() {
                self.windows.camera = true;
            }
            if ui.button("⚙ G-code").clicked() {
                self.windows.gcode = true;
            }
            if ui.button("🔌 Machine").clicked() {
                self.windows.machine = true;
            }
        });
    }

    /// The numeric property bar (X/Y position, width/height of the selection).
    fn coords_bar(&mut self, ui: &mut egui::Ui) {
        let bbox = self.selection_bbox();
        let enabled = bbox.is_some();
        let (ox, oy, ow, oh) = bbox
            .map(|(a, b, c, d)| (a, b, c - a, d - b))
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        let (mut x, mut y, mut w, mut h) = (ox, oy, ow, oh);
        ui.horizontal_wrapped(|ui| {
            ui.label("X");
            ui.add_enabled(enabled, egui::DragValue::new(&mut x).speed(0.2).suffix(" mm"));
            ui.label("Y");
            ui.add_enabled(enabled, egui::DragValue::new(&mut y).speed(0.2).suffix(" mm"));
            ui.separator();
            ui.label("L");
            ui.add_enabled(enabled, egui::DragValue::new(&mut w).speed(0.2).range(0.1..=100_000.0).suffix(" mm"));
            ui.label("H");
            ui.add_enabled(enabled, egui::DragValue::new(&mut h).speed(0.2).range(0.1..=100_000.0).suffix(" mm"));
            ui.separator();
            ui.label("Zoom");
            ui.add(egui::DragValue::new(&mut self.px_per_mm).range(0.5..=20.0).speed(0.1).suffix(" px/mm"));
            ui.separator();
            ui.checkbox(&mut self.snap, "Aimanter 1 mm");
            ui.separator();
            ui.label(format!("Calque actif : {:02}", self.active_layer));
        });
        if enabled {
            if (x - ox).abs() > 1e-9 {
                self.move_selection(x - ox, 0.0);
            }
            if (y - oy).abs() > 1e-9 {
                self.move_selection(0.0, y - oy);
            }
            if ow > 1e-6 && (w - ow).abs() > 1e-9 {
                self.scale_about(w / ow, 1.0, ox, oy);
            }
            if oh > 1e-6 && (h - oh).abs() > 1e-9 {
                self.scale_about(1.0, h / oh, ox, oy);
            }
        }
    }

    /// The left vertical tool palette (Select / Rect / Ellipse / Pen / Node).
    fn tool_palette(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.vertical_centered(|ui| {
            for t in [
                Tool::Select,
                Tool::Rect,
                Tool::Ellipse,
                Tool::Polygon,
                Tool::Pen,
                Tool::Node,
                Tool::Measure,
            ] {
                let resp = ui.selectable_label(self.tool == t, egui::RichText::new(t.icon()).size(20.0));
                if resp.on_hover_text(t.label()).clicked() {
                    self.tool = t;
                }
            }
            ui.separator();
            if ui.button(egui::RichText::new("A").size(18.0)).on_hover_text("Ajouter du texte").clicked() {
                self.insert_text();
            }
            if ui.button(egui::RichText::new("⧉").size(16.0)).on_hover_text("Dupliquer (Ctrl+D)").clicked() {
                self.duplicate_selection();
            }
            if ui.button(egui::RichText::new("◎").size(16.0)).on_hover_text("Décaler le contour").clicked() {
                self.offset_selection(self.offset_dist);
            }
        });
    }

    /// The right Cuts / Layers dock (table + parameters of the active layer).
    fn cuts_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.strong("Cuts / Calques");
            if ui.small_button("➕").on_hover_text("Ajouter un calque").clicked() {
                let idx = self.project.layers.iter().map(|l| l.index).max().map(|m| m + 1).unwrap_or(0);
                self.project.layers.push(CutLayer::new(idx));
            }
            if ui.small_button("⤢").on_hover_text("Éditer tous les calques").clicked() {
                self.windows.cut_settings = true;
            }
        });
        ui.separator();

        let active_layer = self.active_layer;
        let mut pick: Option<u32> = None;
        egui::Grid::new("cuts_grid")
            .striped(true)
            .num_columns(4)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                for h in ["#", "Nom", "Mode", "Vit/Puis"] {
                    ui.strong(h);
                }
                ui.end_row();
                for i in 0..self.project.layers.len() {
                    let (r, g, b) = self.project.layers[i].rgb();
                    let slot = self.project.layers[i].index;
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 16.0), egui::Sense::click());
                    ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(r, g, b));
                    let txt = if (r as u16 + g as u16 + b as u16) > 380 {
                        egui::Color32::BLACK
                    } else {
                        egui::Color32::WHITE
                    };
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{slot:02}"),
                        egui::FontId::proportional(11.0),
                        txt,
                    );
                    if slot == active_layer {
                        ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 140, 0)), egui::StrokeKind::Middle);
                    }
                    if resp.clicked() {
                        pick = Some(slot);
                    }

                    let layer = &mut self.project.layers[i];
                    ui.add(egui::TextEdit::singleline(&mut layer.name).desired_width(80.0));
                    egui::ComboBox::from_id_salt(("cut_kind", i))
                        .selected_text(layer.kind.label())
                        .width(66.0)
                        .show_ui(ui, |ui| {
                            for k in LayerType::ALL {
                                ui.selectable_value(&mut layer.kind, k, k.label());
                            }
                        });
                    ui.label(format!("{:.0}/{:.0}%", layer.speed, layer.max_power));
                    ui.end_row();
                }
            });

        if let Some(slot) = pick {
            self.active_layer = slot;
            self.status = format!("Calque {slot:02} sélectionné");
        }

        ui.separator();
        if let Some(li) = self.project.layers.iter().position(|l| l.index == self.active_layer) {
            let layer = &mut self.project.layers[li];
            ui.strong(format!("Paramètres — calque {:02}", layer.index));
            egui::Grid::new("cut_params").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Vitesse");
                ui.add(egui::DragValue::new(&mut layer.speed).range(0.0..=10_000.0).speed(1.0).suffix(" mm/s"));
                ui.end_row();
                ui.label("Puissance max");
                ui.add(egui::DragValue::new(&mut layer.max_power).range(0.0..=100.0).speed(0.5).suffix(" %"));
                ui.end_row();
                ui.label("Puissance min");
                ui.add(egui::DragValue::new(&mut layer.min_power).range(0.0..=100.0).speed(0.5).suffix(" %"));
                ui.end_row();
                ui.label("Passes");
                ui.add(egui::DragValue::new(&mut layer.passes).range(1..=50).speed(0.1));
                ui.end_row();
            });
        } else {
            ui.weak("Aucun calque. Clique ➕ pour en créer un.");
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(format!("Machines : {}", self.project.devices.len()));
            if ui.button("Configurer…").clicked() {
                self.windows.devices = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("⚙ G-code…").clicked() {
                self.windows.gcode = true;
            }
            if ui.button("🔌 Console laser…").clicked() {
                self.windows.machine = true;
            }
        });

        ui.separator();
        ui.label("Fichier :");
        ui.add(egui::TextEdit::singleline(&mut self.io_path).desired_width(f32::INFINITY));
        ui.horizontal(|ui| {
            if ui.button("Exporter").clicked() {
                self.export_lbrn2();
            }
            if ui.button("Importer").clicked() {
                self.import_file();
            }
        });
    }

    /// The bottom layer-colour strip (00–29) plus the status line.
    fn swatch_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let active = self.active_layer;
            let mut pick: Option<u32> = None;
            for slot in 0..30u32 {
                let (r, g, b) = layer_rgb(slot);
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 16.0), egui::Sense::click());
                ui.painter().rect_filled(rect, 1.0, egui::Color32::from_rgb(r, g, b));
                if slot == active {
                    ui.painter().rect_stroke(rect, 1.0, egui::Stroke::new(2.0, egui::Color32::WHITE), egui::StrokeKind::Middle);
                }
                if resp.on_hover_text(format!("Calque {slot:02}")).clicked() {
                    pick = Some(slot);
                }
            }
            if let Some(slot) = pick {
                self.set_active_slot(slot);
            }
        });
        ui.horizontal(|ui| {
            ui.label("ℹ");
            ui.label(egui::RichText::new(&self.status).small());
            ui.separator();
            ui.weak(format!("{} forme(s)", self.project.shapes.len()));
        });
    }

    /// Render the floating tool windows (G-code, laser console, layer editor,
    /// machines, preferences, about) — LightBurn's dockable windows.
    fn floating_windows(&mut self, ctx: &egui::Context) {
        let mut open = self.windows.gcode;
        if open {
            egui::Window::new("Parcours G-code")
                .open(&mut open)
                .default_width(540.0)
                .vscroll(true)
                .resizable(true)
                .show(ctx, |ui| self.gcode_tab(ui));
        }
        self.windows.gcode = open;

        let mut open = self.windows.machine;
        if open {
            egui::Window::new("Console laser (GRBL)")
                .open(&mut open)
                .default_width(460.0)
                .vscroll(true)
                .resizable(true)
                .show(ctx, |ui| self.machine_tab(ui));
        }
        self.windows.machine = open;

        let mut open = self.windows.camera;
        if open {
            egui::Window::new("Caméra / tracer une image")
                .open(&mut open)
                .default_width(470.0)
                .vscroll(true)
                .resizable(true)
                .show(ctx, |ui| self.camera_tab(ui));
        }
        self.windows.camera = open;

        let mut open = self.windows.cut_settings;
        if open {
            egui::Window::new("Réglages des calques")
                .open(&mut open)
                .default_width(640.0)
                .vscroll(true)
                .resizable(true)
                .show(ctx, |ui| self.layers_tab(ui));
        }
        self.windows.cut_settings = open;

        let mut open = self.windows.devices;
        if open {
            egui::Window::new("Machines")
                .open(&mut open)
                .default_width(520.0)
                .vscroll(true)
                .resizable(true)
                .show(ctx, |ui| self.devices_tab(ui));
        }
        self.windows.devices = open;

        let mut open = self.windows.prefs;
        if open {
            egui::Window::new("Préférences")
                .open(&mut open)
                .default_width(380.0)
                .resizable(true)
                .show(ctx, |ui| self.prefs_tab(ui));
        }
        self.windows.prefs = open;

        let mut open = self.windows.about;
        if open {
            egui::Window::new("À propos de BrûleLumière")
                .open(&mut open)
                .default_width(360.0)
                .show(ctx, |ui| {
                    ui.heading("BrûleLumière");
                    ui.label("Éditeur de projet laser inspiré de LightBurn.");
                    ui.separator();
                    ui.label("Calques de découpe, formes vectorielles, édition de nœuds,");
                    ui.label("transformations, import/export .lbrn2 · SVG · DXF,");
                    ui.label("texte vectoriel, tableaux, opérations booléennes, vectorisation d'image,");
                    ui.label("génération G-code GRBL et connexion machine série.");
                    ui.separator();
                    ui.weak("Stockage : project.json (atomique, sauvegardes, garde de version).");
                });
        }
        self.windows.about = open;
    }
}

/// Which bezier handle of which vertex a drag targets.
#[derive(Clone, Copy)]
enum HandleRef {
    C0(usize),
    C1(usize),
}

/// Alignment of a multi-selection within its bounding box.
#[derive(Clone, Copy)]
enum Align {
    Left,
    Right,
    Top,
    Bottom,
    CenterH,
    CenterV,
}

impl Align {
    fn label(self) -> &'static str {
        match self {
            Align::Left => "gauche",
            Align::Right => "droite",
            Align::Top => "haut",
            Align::Bottom => "bas",
            Align::CenterH => "centre horizontal",
            Align::CenterV => "centre vertical",
        }
    }
}

/// Offset a closed ring of points outward (positive `dist`) or inward
/// (negative) along each vertex's averaged edge normal. This is the simple
/// normal-offset used for a demo "Offset Shapes": good for convex-ish outlines,
/// it can self-intersect on sharp concavities — enough to show the feature.
fn offset_ring(pts: &[(f64, f64)], dist: f64, closed: bool) -> Vec<(f64, f64)> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    // Signed area to orient normals so positive `dist` grows the outline.
    let mut area = 0.0;
    for i in 0..n {
        let (x0, y0) = pts[i];
        let (x1, y1) = pts[(i + 1) % n];
        area += x0 * y1 - x1 * y0;
    }
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let prev = pts[(i + n - 1) % n];
        let cur = pts[i];
        let next = pts[(i + 1) % n];
        // Outward normals of the two incident edges (rotate edge by -90°).
        let n1 = edge_normal(prev, cur);
        let n2 = edge_normal(cur, next);
        let (mut nx, mut ny) = (n1.0 + n2.0, n1.1 + n2.1);
        let len = (nx * nx + ny * ny).sqrt();
        if len < 1e-9 {
            out.push(cur);
            continue;
        }
        nx /= len;
        ny /= len;
        // Miter: move far enough along the averaged normal that both edges are
        // offset by `dist`. `cos_half = m·n1`; clamp very sharp corners.
        let cos_half = nx * n1.0 + ny * n1.1;
        let scale = if cos_half.abs() < 0.2 { dist * 5.0 } else { dist / cos_half };
        out.push((cur.0 + sign * scale * nx, cur.1 + sign * scale * ny));
    }
    let _ = closed;
    out
}

/// Unit outward normal of the edge a→b (edge direction rotated by -90°).
fn edge_normal(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        return (0.0, 0.0);
    }
    (dy / len, -dx / len)
}

/// Apply the inverse of an [`XForm`]'s linear part to a delta vector, so a
/// screen-space drag maps back to the shape's local coordinates.
fn inverse_linear(x: &XForm, dx: f64, dy: f64) -> (f64, f64) {
    let det = x.a * x.d - x.b * x.c;
    if det.abs() < 1e-12 {
        return (dx, dy);
    }
    let inv = 1.0 / det;
    // Inverse of [[a c] [b d]] is 1/det * [[d -c] [-b a]].
    ((x.d * dx - x.c * dy) * inv, (-x.b * dx + x.a * dy) * inv)
}

type P = (f64, f64);

fn lerp(a: P, b: P, t: f64) -> P {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// de Casteljau split of a cubic at `t`. Returns the left control triple
/// `(p0, a, d)`, the split point `m`, and the right control triple
/// `(e, c, p3)`.
fn split_cubic(p0: P, p1: P, p2: P, p3: P, t: f64) -> ((P, P, P), P, (P, P, P)) {
    let a = lerp(p0, p1, t);
    let b = lerp(p1, p2, t);
    let c = lerp(p2, p3, t);
    let d = lerp(a, b, t);
    let e = lerp(b, c, t);
    let m = lerp(d, e, t);
    ((p0, a, d), m, (e, c, p3))
}

/// Draw a 10 mm reference grid, with the model axes slightly darker.
fn draw_grid(painter: &egui::Painter, rect: egui::Rect, origin: egui::Pos2, s: f32) {
    let step_px = 10.0 * s;
    if step_px < 3.0 {
        return;
    }
    let grid = egui::Color32::from_gray(230);
    let axis = egui::Color32::from_gray(170);

    let mut x = origin.x;
    while x <= rect.right() {
        let col = if (x - origin.x).abs() < 0.5 { axis } else { grid };
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, col),
        );
        x += step_px;
    }
    let mut y = origin.y;
    while y <= rect.bottom() {
        let col = if (y - origin.y).abs() < 0.5 { axis } else { grid };
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, col),
        );
        y += step_px;
    }
}

impl eframe::App for BruleLumiereApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Keyboard shortcuts — skipped while a text field holds focus so they
        // don't hijack typing (Ctrl+C etc. still edit text there).
        if !ui.ctx().egui_wants_keyboard_input() {
            let (cmd, shift, key) = ui.input(|i| {
                let k = [
                    egui::Key::S,
                    egui::Key::Z,
                    egui::Key::Y,
                    egui::Key::C,
                    egui::Key::X,
                    egui::Key::V,
                    egui::Key::D,
                    egui::Key::A,
                ]
                .into_iter()
                .find(|&k| i.key_pressed(k));
                (i.modifiers.command, i.modifiers.shift, k)
            });
            if cmd {
                match key {
                    Some(egui::Key::S) => self.save(),
                    Some(egui::Key::Z) if shift => self.redo(),
                    Some(egui::Key::Z) => self.undo(),
                    Some(egui::Key::Y) => self.redo(),
                    Some(egui::Key::C) => self.copy_selection(),
                    Some(egui::Key::X) => self.cut_selection(),
                    Some(egui::Key::V) => self.paste(),
                    Some(egui::Key::D) => self.duplicate_selection(),
                    Some(egui::Key::A) => self.select_all(),
                    _ => {}
                }
            }
        }
        let ctx = ui.ctx().clone();
        // Keep the laser console log/progress ticking while connected.
        if self.machine.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        egui::Panel::top("rea_menubar").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("rea_toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::top("rea_coords").show(ui, |ui| self.coords_bar(ui));
        egui::Panel::bottom("rea_swatches").show(ui, |ui| self.swatch_bar(ui));
        egui::Panel::left("rea_tools")
            .resizable(false)
            .exact_size(48.0)
            .show(ui, |ui| self.tool_palette(ui));
        egui::Panel::right("rea_cuts")
            .resizable(true)
            .default_size(290.0)
            .min_size(230.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.cuts_panel(ui));
            });
        egui::CentralPanel::default().show(ui, |ui| {
            self.context_bar(ui);
            ui.separator();
            self.canvas_surface(ui);
        });

        self.floating_windows(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::{Prim, Shape, ShapeKind};

    fn app_with_open_path() -> BruleLumiereApp {
        let dir = std::env::temp_dir().join(format!(
            "rea-app-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut app = BruleLumiereApp::new(dir);
        app.project.shapes.clear();
        app.project
            .shapes
            .push(Shape::polyline(0, &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], false));
        app.selected = Some(0);
        app.selected_node = Some(0);
        app
    }

    fn path(app: &BruleLumiereApp) -> (&Vec<crate::shapes::Vertex>, &Vec<Prim>) {
        match &app.project.shapes[0].kind {
            ShapeKind::Path { verts, prims, .. } => (verts, prims),
            _ => panic!("not a path"),
        }
    }

    #[test]
    fn curve_then_corner_toggles_handles() {
        let mut app = app_with_open_path();
        app.curve_segment_after_node();
        {
            let (verts, prims) = path(&app);
            assert!(matches!(prims[0], Prim::Bezier { a: 0, b: 1 }));
            assert!(verts[0].c1.is_some());
            assert!(verts[1].c0.is_some());
        }
        app.make_node_corner();
        let (verts, prims) = path(&app);
        assert!(matches!(prims[0], Prim::Line { a: 0, b: 1 }));
        assert!(verts[0].c1.is_none());
    }

    #[test]
    fn split_inserts_vertex_and_primitive() {
        let mut app = app_with_open_path();
        let before = path(&app).1.len();
        app.split_after_node();
        let (verts, prims) = path(&app);
        assert_eq!(verts.len(), 4);
        assert_eq!(prims.len(), before + 1);
        // The split midpoint of segment (0,0)-(10,0) is (5,0).
        assert_eq!(verts[3].x, 5.0);
        assert_eq!(verts[3].y, 0.0);
    }

    #[test]
    fn delete_middle_node_reconnects_neighbours() {
        let mut app = app_with_open_path();
        app.selected_node = Some(1);
        app.delete_node();
        let (verts, prims) = path(&app);
        assert_eq!(verts.len(), 2);
        // The reconnection links the two surviving endpoints.
        assert!(prims.iter().any(|p| matches!(p, Prim::Line { a: 0, b: 1 })));
    }

    #[test]
    fn split_cubic_midpoint_on_curve() {
        let (_, mid, _) = split_cubic((0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0), 0.5);
        assert!((mid.0 - 1.5).abs() < 1e-9);
        assert!(mid.1.abs() < 1e-9);
    }

    fn app_with_two_rects() -> BruleLumiereApp {
        let dir = std::env::temp_dir().join(format!(
            "rea-app-tx-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let mut app = BruleLumiereApp::new(dir);
        app.project.shapes.clear();
        app.project.shapes.push(Shape::rect(0, 0.0, 0.0, 10.0, 10.0));
        app.project.shapes.push(Shape::rect(1, 40.0, 0.0, 10.0, 10.0));
        app.sel = vec![0, 1];
        app.selected = Some(1);
        app
    }

    #[test]
    fn group_then_ungroup_roundtrips() {
        let mut app = app_with_two_rects();
        app.group_selection();
        assert_eq!(app.project.shapes.len(), 1);
        assert!(matches!(app.project.shapes[0].kind, ShapeKind::Group { .. }));
        app.selected = Some(0);
        app.ungroup_selection();
        assert_eq!(app.project.shapes.len(), 2);
        assert!(app.project.shapes.iter().all(|s| matches!(s.kind, ShapeKind::Rect { .. })));
    }

    #[test]
    fn move_selection_translates() {
        let mut app = app_with_two_rects();
        app.sel = vec![0];
        app.move_selection(5.0, 3.0);
        let bb = app.project.shapes[0].world_bbox(16).unwrap();
        assert!((bb.0 - 5.0).abs() < 1e-9 && (bb.1 - 3.0).abs() < 1e-9);
    }

    #[test]
    fn snap_rounds_translation() {
        let mut app = app_with_two_rects();
        app.sel = vec![0];
        app.move_selection(4.6, 2.2);
        app.snap_selection();
        assert_eq!(app.project.shapes[0].xform.e, 5.0);
        assert_eq!(app.project.shapes[0].xform.f, 2.0);
    }

    #[test]
    fn undo_restores_after_delete() {
        let mut app = app_with_two_rects();
        app.sel = vec![0, 1];
        app.delete_selection();
        assert_eq!(app.project.shapes.len(), 0);
        app.undo();
        assert_eq!(app.project.shapes.len(), 2);
        app.redo();
        assert_eq!(app.project.shapes.len(), 0);
    }

    #[test]
    fn copy_paste_adds_offset_copies() {
        let mut app = app_with_two_rects();
        app.sel = vec![0];
        app.copy_selection();
        app.paste();
        assert_eq!(app.project.shapes.len(), 3);
        // The pasted copy is the new selection, shifted by (5,5) mm.
        let bb = app.project.shapes[2].world_bbox(16).unwrap();
        assert!((bb.0 - 5.0).abs() < 1e-9 && (bb.1 - 5.0).abs() < 1e-9);
    }

    #[test]
    fn duplicate_selects_the_copy() {
        let mut app = app_with_two_rects();
        app.sel = vec![0];
        app.duplicate_selection();
        assert_eq!(app.project.shapes.len(), 3);
        assert_eq!(app.sel, vec![2]);
    }

    #[test]
    fn align_left_lines_up_edges() {
        let mut app = app_with_two_rects();
        // rect 0 at x=0, rect 1 at x=40; aligning left moves rect 1 to x=0.
        app.sel = vec![0, 1];
        app.align_selection(Align::Left);
        let bb = app.project.shapes[1].world_bbox(16).unwrap();
        assert!((bb.0 - 0.0).abs() < 1e-9, "left edge should be 0, got {}", bb.0);
    }

    #[test]
    fn offset_ring_grows_square() {
        // A 10 mm CCW square, outset by 1 mm, should span -1..11 on each axis.
        let sq = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let out = offset_ring(&sq, 1.0, true);
        let minx = out.iter().map(|p| p.0).fold(f64::MAX, f64::min);
        let maxx = out.iter().map(|p| p.0).fold(f64::MIN, f64::max);
        assert!((minx + 1.0).abs() < 1e-6, "minx={minx}");
        assert!((maxx - 11.0).abs() < 1e-6, "maxx={maxx}");
    }

    #[test]
    fn polygon_tool_builds_closed_ngon() {
        let mut app = app_with_two_rects();
        app.project.shapes.clear();
        app.tool = Tool::Polygon;
        app.poly_sides = 5;
        app.create_dragged_shape((0.0, 0.0), (20.0, 20.0));
        assert_eq!(app.project.shapes.len(), 1);
        match &app.project.shapes[0].kind {
            ShapeKind::Path { verts, closed, .. } => {
                assert_eq!(verts.len(), 5);
                assert!(*closed);
            }
            _ => panic!("expected a closed path"),
        }
    }
}
