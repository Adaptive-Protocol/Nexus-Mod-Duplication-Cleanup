#![cfg_attr(windows, windows_subsystem = "windows")]
//! Nexus Mod Dedupe - desktop front end for the dedupe engine (Windows & Linux).
mod engine;

use eframe::egui::{self, Align, Layout, RichText};
use engine as nd;
use std::{fs, mem, path::{Path, PathBuf}};

const TITLE: &str = "Nexus Mod Dedupe";
const REVIEW_HELP: &str = "These mods need a judgment call: their file names carry different embedded versions \
    (e.g. CET builds for different game patches), or upload order disagrees with version numbers.\n\n\
    Ticked mods are collapsed to the newest file. Untick any mod whose older files you want to keep, e.g. as a rollback option.";

#[derive(Default)]
enum Dialog { #[default] None, Review(Vec<(usize, bool)>), Confirm, Info(&'static str, String) }

struct Scan { decisions: Vec<nd::Decision>, unparsed: Vec<String>, folder: PathBuf, keep: usize }

struct App { folder: String, keep: usize, hard: bool, scan: Option<Scan>, report: String, dialog: Dialog }

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let exe_dir = std::env::current_exe().ok().and_then(|p| Some(p.parent()?.display().to_string())).unwrap_or_default();
        let (folder, keep) = cc.storage.and_then(|s| eframe::get_value(s, eframe::APP_KEY)).unwrap_or((exe_dir, 1));
        Self { folder, keep, hard: false, scan: None, dialog: Dialog::None,
               report: "Pick your Nexus downloads folder and press Scan. Nothing is changed until you press Clean up.".into() }
    }

    fn scan(&mut self) {
        let folder = PathBuf::from(&self.folder);
        if !folder.is_dir() { return self.dialog = Dialog::Info(TITLE, "Choose an existing folder.".into()) }
        match nd::scan_and_decide(&folder, self.keep) {
            Err(e) => self.dialog = Dialog::Info(TITLE, format!("Could not read the folder: {e}")),
            Ok((decisions, unparsed)) => {
                let reviews: Vec<_> = decisions.iter().enumerate().filter(|(_, d)| d.review).map(|(i, _)| (i, true)).collect();
                self.scan = Some(Scan { decisions, unparsed, folder, keep: self.keep });
                if reviews.is_empty() { self.refresh_report() } else { self.dialog = Dialog::Review(reviews) }
            }
        }
    }

    fn refresh_report(&mut self) {
        if let Some(s) = &self.scan { self.report = nd::build_report(&s.folder, false, self.hard, s.keep, &s.decisions, &s.unparsed) }
    }

    fn clean(&mut self) {
        let Some(s) = self.scan.take() else { return };
        let report = nd::build_report(&s.folder, true, self.hard, s.keep, &s.decisions, &s.unparsed);
        let log = s.folder.join(format!("dedupe_log_{}.txt", chrono::Local::now().format("%Y%m%d_%H%M%S")));
        if let Err(e) = fs::write(&log, report) { return self.dialog = Dialog::Info("Nothing changed", format!("Could not write the log file: {e}")) }
        let (ok, fail) = nd::execute(&s.decisions, &s.folder, self.hard);
        self.dialog = Dialog::Info("Done", format!("{ok} files {}, {fail} failures.\nA log was saved in the mods folder.",
                                                   if self.hard { "deleted" } else { "moved to _outdated_trash" }));
    }

    /// Draws the open dialog; returns false once it should close.
    fn show_dialog(&mut self, ctx: &egui::Context, dialog: &mut Dialog) -> bool {
        let modal = |id, add: &mut dyn FnMut(&mut egui::Ui) -> bool| {
            let r = egui::Modal::new(egui::Id::new(id)).show(ctx, |ui| { ui.set_max_width(760.0); add(ui) });
            r.inner && !r.should_close()
        };
        match dialog {
            Dialog::None => false,
            Dialog::Review(rows) => {
                let open = modal("review", &mut |ui| {
                    ui.heading(format!("Review {} judgment calls", rows.len()));
                    ui.label(REVIEW_HELP);
                    ui.separator();
                    let s = &self.scan.as_ref().unwrap().decisions;
                    egui::ScrollArea::both().max_height(380.0).show(ui, |ui| for (i, on) in rows.iter_mut() {
                        let d = &s[*i];
                        ui.checkbox(on, RichText::new(format!("{}  ({} older)  -  {}", d.key_str(), d.delete().len(), d.names.join(" | "))).monospace());
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Select all").clicked() { rows.iter_mut().for_each(|r| r.1 = true) }
                        if ui.button("Select none").clicked() { rows.iter_mut().for_each(|r| r.1 = false) }
                        !ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.button("Continue").clicked()).inner
                    }).inner
                });
                if !open {                                     // unticked mods keep every file
                    let s = self.scan.as_mut().unwrap();
                    for (i, on) in rows.iter() { s.decisions[*i].collapse = *on }
                    self.refresh_report();
                }
                open
            }
            Dialog::Confirm => {
                let (n, size) = nd::totals(&self.scan.as_ref().unwrap().decisions);
                let verb = if self.hard { "PERMANENTLY DELETE" } else { "Move to _outdated_trash" };
                let mut yes = false;
                let open = modal("confirm", &mut |ui| {
                    ui.heading("Confirm");
                    ui.label(format!("{verb} {n} outdated files ({})?", nd::fmt_size(size)));
                    ui.horizontal(|ui| { yes = ui.button("Yes").clicked(); !yes && !ui.button("No").clicked() }).inner
                });
                if yes { self.clean() }
                open && !yes
            }
            Dialog::Info(title, text) => modal("info", &mut |ui| {
                ui.heading(*title);
                ui.label(text.as_str());
                !ui.button("OK").clicked()
            }),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::TopBottomPanel::top("bar").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Mods folder:");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Browse…").clicked() && let Some(d) = rfd::FileDialog::new().set_directory(&self.folder).pick_folder() {
                        self.folder = d.display().to_string()
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.folder).desired_width(f32::INFINITY));
                });
            });
            ui.horizontal(|ui| {
                ui.label("Keep newest per mod:");
                ui.add(egui::DragValue::new(&mut self.keep).range(1..=9));
                ui.checkbox(&mut self.hard, "Delete permanently (skip _outdated_trash)");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ready = self.scan.as_ref().is_some_and(|s| s.keep == self.keep && nd::totals(&s.decisions).0 > 0);
                    if ui.add_enabled(ready, egui::Button::new("Clean up")).clicked() { self.dialog = Dialog::Confirm }
                    if ui.button("Scan").clicked() { self.scan() }
                });
            });
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| ui.add(egui::Label::new(RichText::new(&self.report).monospace()).extend()));
        });
        let mut dialog = mem::take(&mut self.dialog);
        if self.show_dialog(ctx, &mut dialog) && matches!(self.dialog, Dialog::None) { self.dialog = dialog }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) { eframe::set_value(storage, eframe::APP_KEY, &(&self.folder, self.keep)) }
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
    let opts = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_title(TITLE).with_inner_size([960.0, 620.0]), ..Default::default() };
    eframe::run_native(TITLE, opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
