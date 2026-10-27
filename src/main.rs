#![cfg_attr(windows, windows_subsystem = "windows")]
//! Nexus Mod Dedupe - desktop front end for the dedupe engine (Windows & Linux).
mod engine;

use eframe::egui::{self, Align, Color32, Layout, RichText, ThemePreference};
use engine::{self as nd, Decision};
use std::{collections::HashMap, fs, io, mem, path::{Path, PathBuf}, thread::{self, JoinHandle}, time::Duration};

const TITLE: &str = "Nexus Mod Dedupe";   // also names the settings folder, which the uninstaller removes (APP_ID there)
const REVIEW_HELP: &str = "These mods need a judgment call: their file names carry different embedded versions \
    (e.g. CET builds for different game patches), or upload order disagrees with version numbers.\n\n\
    Ticked mods are collapsed to the newest file. Untick any mod whose older files you want to keep, e.g. as a rollback option.";
const ABOUT: &str = concat!("Nexus Mod Dedupe v", env!("CARGO_PKG_VERSION"), ", MIT license. It works offline and never connects to the internet.\n\n\
    Design inspired by red4-conflicts by rfuzzo (github.com/rfuzzo/Cyberpunk-utility): the dark theme and theme switch, \
    the mod rows with coloured status circles, the mod filter and the summary line. \
    The status groups, start page, judgment-call popup and clean-up rules are this app's own.\n\n\
    Built with egui/eframe, rfd, fancy-regex and chrono. Their licenses are in THIRD-PARTY-NOTICES.txt.\n\n\
    The code was written by an AI assistant (Claude, made by Anthropic) at the maintainer's direction.");

#[derive(Default)]
enum Dialog { #[default] None, Review(Vec<(usize, bool)>), Confirm, Info(&'static str, String) }

struct Scan { decisions: Vec<Decision>, unparsed: Vec<String>, folder: PathBuf, keep: usize }

type Job = (PathBuf, usize, JoinHandle<io::Result<(Vec<Decision>, Vec<String>)>>);

struct App {
    folder: String, keep: usize, hard: bool, filter: String,
    scan: Option<Scan>,                          // last scan that found mods; None shows the start page
    job: Option<Job>,                            // scan running in the background
    note: String,                                // why the start page is showing
    choices: HashMap<(PathBuf, String), bool>,   // judgment calls answered this session
    dialog: Dialog,
    theme: Option<ThemePreference>,              // last theme sent to the native title bar
}

impl App {
    fn new(cc: &eframe::CreationContext, start: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_theme(cc.storage.and_then(|s| eframe::get_value(s, "theme")).unwrap_or(ThemePreference::Dark));   // dark until the user picks otherwise
        let (folder, keep) = cc.storage.and_then(|s| eframe::get_value(s, eframe::APP_KEY)).unwrap_or((String::new(), 1));
        let mut app = Self { folder, keep, hard: false, filter: String::new(), scan: None, job: None, note: String::new(),
                             choices: HashMap::new(), dialog: Dialog::None, theme: None };
        let dir = start.unwrap_or_else(|| PathBuf::from(&app.folder));
        app.open(dir);
        app
    }

    /// Switches to `dir`: the start page shows until its scan finishes.
    fn open(&mut self, dir: PathBuf) {
        self.folder = dir.display().to_string();
        self.scan = None;
        self.rescan();
    }

    /// Scans the folder on a background thread, so a slow drive never freezes the window.
    fn rescan(&mut self) {
        let (dir, keep) = (PathBuf::from(self.folder.trim().trim_matches('"')), self.keep);   // tolerates a pasted "quoted path"
        if dir.as_os_str().is_empty() { return (self.scan, self.job, self.note) = (None, None, String::new()) }
        let d = dir.clone();
        self.job = Some((dir, keep, thread::spawn(move || nd::scan_and_decide(&d, keep))));
    }

    fn browse(&mut self) {
        let dlg = rfd::FileDialog::new();
        let dlg = if Path::new(&self.folder).is_dir() { dlg.set_directory(&self.folder) } else { dlg };
        if let Some(d) = dlg.pick_folder() { self.open(d) }
    }

    /// Takes in a finished scan. Waits while a dialog is open, so the review list never points at stale results.
    fn poll(&mut self) {
        if !matches!(self.dialog, Dialog::None) || !self.job.as_ref().is_some_and(|j| j.2.is_finished()) { return }
        let (folder, keep, job) = self.job.take().unwrap();
        match job.join().unwrap_or_else(|_| Err(io::Error::other("the scan stopped unexpectedly"))) {
            Ok((mut decisions, unparsed)) if !decisions.is_empty() => {
                let mut ask = vec![];
                for (i, d) in decisions.iter_mut().enumerate().filter(|(_, d)| d.review) {
                    match self.choices.get(&(folder.clone(), d.key_str())) { Some(&on) => d.collapse = on, None => ask.push((i, true)) }
                }
                if !ask.is_empty() { self.dialog = Dialog::Review(ask) }
                self.scan = Some(Scan { decisions, unparsed, folder, keep });
            }
            r => {
                let shown = folder.display();
                self.scan = None;
                self.note = match r {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => format!("Folder not found: {shown}"),
                    Err(e) => format!("Could not read {shown}: {e}"),
                    Ok(_) => format!("No Nexus mod downloads found in {shown}."),
                };
            }
        }
    }

    fn clean(&mut self) {
        let Some(s) = self.scan.take() else { return };
        let report = nd::build_report(&s.folder, true, self.hard, s.keep, &s.decisions, &s.unparsed);
        let log = s.folder.join(format!("dedupe_log_{}.txt", chrono::Local::now().format("%Y%m%d_%H%M%S")));
        self.dialog = match fs::write(&log, report) {
            Err(e) => Dialog::Info("Nothing changed", format!("Could not write the log file: {e}")),
            Ok(()) => {
                let (ok, fail) = nd::execute(&s.decisions, &s.folder, self.hard);
                Dialog::Info("Done", format!("{} {}, {}.\nA log was saved in the mods folder.", count(ok, "file"),
                                             if self.hard { "deleted" } else { "moved to _outdated_trash" }, count(fail, "failure")))
            }
        };
        self.folder = s.folder.display().to_string();
        self.rescan();
    }

    /// The first page: the chosen folder's mods, grouped by status.
    fn lists(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.scan.as_mut() else { return };
        ui.horizontal(|ui| {
            ui.label("Mod filter:");
            ui.add(egui::TextEdit::singleline(&mut self.filter).desired_width(300.0));
            if ui.button("x").clicked() { self.filter.clear() }
        });
        let (n, size) = nd::totals(&s.decisions);
        let mods = s.decisions.iter().filter(|d| !d.delete().is_empty()).count();
        ui.label(if n == 0 { "Nothing to clean up: every mod is at its newest file.".into() }
                 else { format!("Found {} across {} ({} to free)", count(n, "outdated file"), count(mods, "mod"), nd::fmt_size(size)) });
        ui.separator();
        let q = self.filter.to_lowercase();
        let mut rows: Vec<usize> = (0..s.decisions.len()).filter(|&i| s.decisions[i].key_str().to_lowercase().contains(&q)).collect();
        rows.sort_by_cached_key(|&i| s.decisions[i].key_str().to_lowercase());
        let (review, rest): (Vec<_>, Vec<_>) = rows.into_iter().partition(|&i| s.decisions[i].review);
        let (old, current): (Vec<_>, Vec<_>) = rest.into_iter().partition(|&i| !s.decisions[i].delete().is_empty());
        let other: Vec<&String> = s.unparsed.iter().filter(|n| n.to_lowercase().contains(&q)).collect();
        let (n, size) = old.iter().flat_map(|&i| s.decisions[i].delete()).fold((0, 0), |(n, b), f| (n + 1, b + f.size));
        let ticked = review.iter().filter(|&&i| s.decisions[i].collapse).count();
        let hard = self.hard;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            group(ui, "old", Color32::RED, format!("Outdated: {} in {} ({})", count(n, "file"), count(old.len(), "mod"), nd::fmt_size(size)), old.len(), true,
                  |ui| for &i in &old { mod_row(ui, &mut s.decisions[i], hard); });
            group(ui, "current", Color32::GREEN, format!("Up to date: {}", count(current.len(), "mod")), current.len(), false,
                  |ui| for &i in &current { mod_row(ui, &mut s.decisions[i], hard); });
            group(ui, "review", Color32::GRAY, format!("Needs review: {} ({ticked} ticked)", count(review.len(), "mod")), review.len(), true,
                  |ui| for &i in &review {
                      let d = &mut s.decisions[i];
                      if mod_row(ui, d, hard) { self.choices.insert((s.folder.clone(), d.key_str()), d.collapse); }
                  });
            group(ui, "other", Color32::GRAY, format!("Not recognised: {}", count(other.len(), "file")), other.len(), false,
                  |ui| for f in &other { ui.horizontal(|ui| { dot(ui, Color32::GRAY, "Not a Nexus download name: left alone"); ui.label(f.as_str()) }); });
            if old.len() + current.len() + review.len() + other.len() == 0 { ui.weak("No mods match the filter."); }
        });
    }

    fn start_page(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.heading("Choose your Nexus downloads folder to begin");
            ui.label("Nothing is changed until you press Clean up.");
            ui.add_space(12.0);
            if ui.add(egui::Button::new(RichText::new("Browse…").size(18.0)).min_size(egui::vec2(160.0, 36.0))).clicked() { self.browse() }
            ui.add_space(6.0);
            ui.weak("or drop the folder onto this window");
            if !self.note.is_empty() { ui.add_space(12.0); ui.weak(&self.note); }
        });
    }

    /// Draws the open dialog; returns false once it should close.
    fn show_dialog(&mut self, ctx: &egui::Context, dialog: &mut Dialog) -> bool {
        let modal = |id, add: &mut dyn FnMut(&mut egui::Ui) -> bool| {
            let r = egui::Modal::new(egui::Id::new(id)).show(ctx, |ui| { ui.set_max_width(760.0); add(ui) });
            r.inner && !r.should_close()
        };
        if let Dialog::Info(title, text) = dialog {
            return modal("info", &mut |ui| { ui.heading(*title); ui.label(text.as_str()); !ui.button("OK").clicked() });
        }
        let Some(s) = self.scan.as_mut() else { return false };
        match dialog {
            Dialog::Review(rows) => {
                let open = modal("review", &mut |ui| {
                    ui.heading(format!("Review {} judgment calls", rows.len()));
                    ui.label(REVIEW_HELP);
                    ui.separator();
                    egui::ScrollArea::both().max_height(380.0).show(ui, |ui| for (i, on) in rows.iter_mut() {
                        let d = &s.decisions[*i];
                        ui.checkbox(on, RichText::new(format!("{}  ({} older)  -  {}", d.key_str(), d.files.len().saturating_sub(s.keep), d.names.join(" | "))).monospace());
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Select all").clicked() { rows.iter_mut().for_each(|r| r.1 = true) }
                        if ui.button("Select none").clicked() { rows.iter_mut().for_each(|r| r.1 = false) }
                        !ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.button("Continue").clicked()).inner
                    }).inner
                });
                if !open {                                     // unticked mods keep every file
                    for &(i, on) in rows.iter() {
                        s.decisions[i].collapse = on;
                        self.choices.insert((s.folder.clone(), s.decisions[i].key_str()), on);
                    }
                }
                open
            }
            Dialog::Confirm => {
                let (n, size) = nd::totals(&s.decisions);
                let verb = if self.hard { "PERMANENTLY DELETE" } else { "Move to _outdated_trash" };
                let mut yes = false;
                let open = modal("confirm", &mut |ui| {
                    ui.heading("Confirm");
                    ui.label(format!("{verb} {} ({})?", count(n, "outdated file"), nd::fmt_size(size)));
                    ui.horizontal(|ui| { yes = ui.button("Yes").clicked(); !yes && !ui.button("No").clicked() }).inner
                });
                if yes { self.clean() }
                open && !yes
            }
            _ => false,
        }
    }
}

/// A 10 px status circle, as in red4-conflicts, with hover text so colour is never the only cue.
fn dot(ui: &mut egui::Ui, color: Color32, tip: &str) {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
    if !tip.is_empty() { r.on_hover_text(tip); }
}

/// "1 file", "2 files".
fn count(n: usize, noun: &str) -> String { format!("{n} {noun}{}", if n == 1 { "" } else { "s" }) }

/// What happens to the i-th newest file of a mod on Clean up.
fn state(d: &Decision, i: usize, hard: bool) -> (Color32, &'static str) {
    match (i < d.keep, d.collapse) {
        (true, _) => (Color32::GREEN, if i == 0 { "Newest: kept" } else { "Kept as a rollback copy" }),
        (false, true) => (Color32::RED, if hard { "Outdated: permanently deleted on Clean up" } else { "Outdated: moved to _outdated_trash on Clean up" }),
        (false, false) => (Color32::GRAY, "Left alone: you unticked this judgment call"),
    }
}

/// A collapsible row; clicking the header text (the `header` result) toggles it too, as in a CollapsingHeader.
fn collapsing(ui: &mut egui::Ui, id: egui::Id, open: bool, header: impl FnOnce(&mut egui::Ui) -> bool, body: impl FnOnce(&mut egui::Ui)) {
    let mut clicked = false;
    let mut h = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open).show_header(ui, |ui| clicked = header(ui));
    if clicked { h.toggle() }
    h.body(body);
}

/// A group headed by a status circle and its counts; hidden when empty.
fn group(ui: &mut egui::Ui, id: &str, color: Color32, title: String, n: usize, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    if n == 0 { return }
    let id = ui.make_persistent_id(id);
    collapsing(ui, id, open, |ui| { dot(ui, color, ""); ui.add(egui::Label::new(RichText::new(title).strong()).sense(egui::Sense::click())).clicked() }, body);
    ui.add_space(4.0);
}

/// One mod, "<name>-<id> (keep K, outdated N)" followed by its status circles; expands to its files, newest first.
/// Judgment calls get a tick box (ticked = remove the older files). Returns true when the tick changed.
fn mod_row(ui: &mut egui::Ui, d: &mut Decision, hard: bool) -> bool {
    let (mut on, id) = (d.collapse, ui.make_persistent_id(d.key_str()));
    let r = &*d;
    collapsing(ui, id, false, |ui| {
        if r.review { ui.checkbox(&mut on, "").on_hover_text("Ticked: older files are removed on Clean up. Unticked: every file is kept."); }
        let (w, color) = ((ui.available_width() - 40.0).max(100.0), ui.visuals().widgets.inactive.text_color());   // room for the circles
        let text = RichText::new(format!("{} (keep {}, outdated {})", r.key_str(), r.kept().len(), r.delete().len())).color(color);
        let clicked = ui.scope(|ui| { ui.set_max_width(w); ui.add(egui::Label::new(text).truncate().sense(egui::Sense::click())).clicked() }).inner;
        let mut shown = vec![];
        for (c, tip) in (0..r.files.len()).map(|i| state(r, i, hard)) { if !shown.contains(&c) { shown.push(c); dot(ui, c, tip) } }
        clicked
    }, |ui| { egui::Grid::new(id).num_columns(5).striped(true).spacing([16.0, 2.0]).show(ui, |ui| {
        for (i, f) in r.files.iter().enumerate() {
            let (c, tip) = state(r, i, hard);
            dot(ui, c, tip);
            for t in [nd::fmt_ver(&f.version), nd::fmt_date(f.ts), nd::fmt_size(f.size)] { ui.label(t); }
            ui.add(egui::Label::new(nd::file_name(&f.path)).truncate());
            ui.end_row();
        }
    }); });
    (on != d.collapse) && { d.collapse = on; true }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        if self.job.is_some() { ctx.request_repaint_after(Duration::from_millis(50)) }
        let pref = ctx.options(|o| o.theme_preference);
        if self.theme != Some(pref) {                       // keep the native title bar in step with the theme
            self.theme = Some(pref);
            ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(match pref {
                ThemePreference::Dark => egui::SystemTheme::Dark,
                ThemePreference::Light => egui::SystemTheme::Light,
                ThemePreference::System => egui::SystemTheme::SystemDefault,
            }));
        }
        if matches!(self.dialog, Dialog::None) && let Some(p) = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone())) {
            let dir = if p.is_dir() { p } else { p.parent().map_or(p.clone(), Path::to_path_buf) };   // a dropped file opens its folder
            self.open(dir);
        }
        egui::TopBottomPanel::top("menu").show(ctx, |ui| egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("About", |ui| ui.vertical(|ui| { ui.set_max_width(440.0); ui.label(ABOUT) }));   // credits and origins, as plain text: no links; vertical() stops menu justification
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.horizontal(egui::widgets::global_theme_preference_buttons);
                ui.separator();
                ui.weak(concat!("v", env!("CARGO_PKG_VERSION")));
            });
        }));
        egui::TopBottomPanel::top("bar").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Mods folder:");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(self.job.is_none(), egui::Button::new("⟳ Rescan")).clicked() { self.rescan() }
                    if ui.button("Browse…").clicked() { self.browse() }
                    let edit = ui.add(egui::TextEdit::singleline(&mut self.folder).desired_width(f32::INFINITY));
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) { self.open(PathBuf::from(&self.folder)) }
                });
            });
            ui.horizontal(|ui| {
                ui.label("Keep newest per mod:");
                if ui.add(egui::DragValue::new(&mut self.keep).range(1..=9)).changed() { self.rescan() }
                ui.checkbox(&mut self.hard, "Delete permanently (skip _outdated_trash)");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ready = self.job.is_none() && self.scan.as_ref().is_some_and(|s| nd::totals(&s.decisions).0 > 0);
                    if ui.add_enabled(ready, egui::Button::new("Clean up")).clicked() { self.dialog = Dialog::Confirm }
                    if self.job.is_some() { ui.spinner(); }
                });
            });
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.scan.is_some() { self.lists(ui) } else if self.job.is_some() { ui.centered_and_justified(|ui| ui.spinner()); } else { self.start_page(ui) }
        });
        let mut dialog = mem::take(&mut self.dialog);
        if self.show_dialog(ctx, &mut dialog) && matches!(self.dialog, Dialog::None) { self.dialog = dialog }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &(&self.folder, self.keep));
        if let Some(t) = self.theme { eframe::set_value(storage, "theme", &t) }
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--report") {          // headless dry run, used by the parity check
        let folder = Path::new(args.get(1).map_or(".", String::as_str));
        let keep = args.get(2).and_then(|k| k.parse().ok()).unwrap_or(1);
        let (d, u) = nd::scan_and_decide(folder, keep).expect("readable folder");
        println!("{}", nd::build_report(folder, false, false, keep, &d, &u));
        return Ok(());
    }
    let start = args.first().map(PathBuf::from).filter(|p| p.is_dir());   // a folder dropped onto the exe
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title(TITLE).with_inner_size([1100.0, 700.0]).with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(TITLE, opts, Box::new(|cc| Ok(Box::new(App::new(cc, start)))))
}
