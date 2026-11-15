use crate::config::*;
use crate::controller::Controller;
use crate::winutil::*;
use eframe::egui;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct App {
    ctl: Arc<Controller>,
    wins: Vec<WinInfo>,
    sel: Option<isize>,
    edit: Option<usize>,
    last_refresh: Instant,
    hotkey_buf: String,
}

impl App {
    pub fn new(ctl: Arc<Controller>) -> Self {
        let hk = ctl.config.lock().unwrap().hotkey.clone();
        Self { ctl, wins: list_windows(), sel: None, edit: None, last_refresh: Instant::now(), hotkey_buf: hk }
    }
}

impl Drop for App {
    fn drop(&mut self) { self.ctl.stop(); }
}

fn slider(ui: &mut egui::Ui, v: &mut f32, r: std::ops::RangeInclusive<f32>, label: &str) {
    ui.add(egui::Slider::new(v, r).text(label));
}

fn settings_ui(ui: &mut egui::Ui, s: &mut Settings) {
    ui.heading("Upscaling");
    egui::ComboBox::from_label("Scaler").selected_text(s.scaler.label()).show_ui(ui, |ui| {
        for v in Scaler::ALL { ui.selectable_value(&mut s.scaler, v, v.label()); }
    });
    egui::ComboBox::from_label("Output size").selected_text(s.fit.label()).show_ui(ui, |ui| {
        for v in FitMode::ALL { ui.selectable_value(&mut s.fit, v, v.label()); }
    });
    if s.fit == FitMode::Custom { slider(ui, &mut s.custom_factor, 1.0..=4.0, "Factor"); }
    slider(ui, &mut s.sharpness, 0.0..=1.0, "Sharpness (adaptive CAS)");
    ui.separator();

    ui.heading("Frame generation");
    egui::ComboBox::from_label("Mode").selected_text(s.frame_gen.label()).show_ui(ui, |ui| {
        for v in FrameGenMode::ALL { ui.selectable_value(&mut s.frame_gen, v, v.label()); }
    });
    if s.frame_gen != FrameGenMode::Off {
        ui.checkbox(&mut s.adaptive, "Adaptive (reach a target FPS)");
        if s.adaptive {
            ui.add(egui::DragValue::new(&mut s.target_fps).range(30..=500).suffix(" FPS target"));
        } else {
            ui.add(egui::Slider::new(&mut s.multiplier, 2..=8).text("Multiplier"));
        }
        egui::ComboBox::from_label("Motion quality").selected_text(s.flow_quality.label()).show_ui(ui, |ui| {
            for v in FlowQuality::ALL { ui.selectable_value(&mut s.flow_quality, v, v.label()); }
        });
        slider(ui, &mut s.artifact_guard, 0.02..=0.3, "Artifact guard (lower = safer)");
    }
    ui.separator();

    ui.heading("Output & latency");
    ui.checkbox(&mut s.vsync, "VSync (off = lowest latency, may tear)");
    ui.checkbox(&mut s.show_osd, "Show FPS counter (source/output)");
    ui.checkbox(&mut s.hide_when_unfocused, "Hide overlay when the game is not focused");
    ui.checkbox(&mut s.remap_cursor, "Remap mouse to scaled image (turn off for raw-input shooters)");
    ui.add(egui::Slider::new(&mut s.scale_delay_secs, 0..=10).text("Start delay (s)"));
    ui.separator();

    ui.heading("Image");
    slider(ui, &mut s.saturation, 0.0..=2.0, "Saturation");
    slider(ui, &mut s.contrast, 0.5..=1.5, "Contrast");
    slider(ui, &mut s.scanlines, 0.0..=1.0, "Scanlines");
    if ui.button("Reset to defaults").clicked() { *s = Settings::default(); }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(250));
        if self.last_refresh.elapsed() > Duration::from_secs(2) {
            self.wins = list_windows();
            self.last_refresh = Instant::now();
        }
        let ctl = self.ctl.clone();
        let mut cfg = ctl.config.lock().unwrap().clone();
        let before = cfg.clone();
        let stats = ctl.stats.lock().unwrap().clone();
        let running = ctl.is_running();
        let mut hotkey_apply = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("FrameForge");
                ui.label("Free real-time upscaler + frame generator. Pick a window (borderless or windowed games work best), then press Scale.");
                ui.separator();

                let sel_label = self
                    .sel
                    .and_then(|h| self.wins.iter().find(|w| w.hwnd == h))
                    .map(|w| format!("{} ({})", w.title, w.exe))
                    .unwrap_or_else(|| "Select a window...".into());
                egui::ComboBox::from_label("Target window").width(380.0).selected_text(sel_label).show_ui(ui, |ui| {
                    for w in &self.wins {
                        ui.selectable_value(&mut self.sel, Some(w.hwnd), format!("{} ({})", w.title, w.exe));
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Refresh list").clicked() { self.wins = list_windows(); }
                    if running {
                        if ui.button("Stop scaling").clicked() { ctl.stop(); }
                    } else if ui.add_enabled(self.sel.is_some(), egui::Button::new("Scale")).clicked() {
                        if let Some(h) = self.sel { ctl.start(h); }
                    }
                });
                ui.label(format!("Hotkey: {} (scales the focused window)", cfg.hotkey));
                ui.horizontal(|ui| {
                    ui.label(format!("Status: {}", stats.status));
                    if stats.active { ui.label(format!("| source {:.0} fps -> output {:.0} fps", stats.src_fps, stats.out_fps)); }
                });
                if let Some(e) = &stats.error { ui.colored_label(egui::Color32::LIGHT_RED, e); }
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("Editing:");
                    if ui.selectable_label(self.edit.is_none(), "Global").clicked() { self.edit = None; }
                    for (i, p) in cfg.profiles.iter().enumerate() {
                        if ui.selectable_label(self.edit == Some(i), &p.name).clicked() { self.edit = Some(i); }
                    }
                });
                match self.edit {
                    Some(i) if i < cfg.profiles.len() => {
                        let p = &mut cfg.profiles[i];
                        ui.horizontal(|ui| {
                            ui.label("Name");
                            ui.text_edit_singleline(&mut p.name);
                            ui.label("Exe");
                            ui.text_edit_singleline(&mut p.exe);
                        });
                        ui.checkbox(&mut p.auto_scale, "Auto-scale when this game is focused");
                        settings_ui(ui, &mut p.settings);
                        if ui.button("Delete profile").clicked() {
                            cfg.profiles.remove(i);
                            self.edit = None;
                        }
                    }
                    _ => {
                        self.edit = None;
                        settings_ui(ui, &mut cfg.global);
                    }
                }
                ui.separator();
                ui.heading("Profiles & hotkey");
                ui.horizontal(|ui| {
                    let can = self.sel.and_then(|h| self.wins.iter().find(|w| w.hwnd == h)).cloned();
                    if ui.add_enabled(can.is_some(), egui::Button::new("New profile from selected window")).clicked() {
                        if let Some(w) = can {
                            cfg.profiles.push(Profile { name: w.exe.clone(), exe: w.exe.clone(), auto_scale: true, settings: cfg.global });
                            self.edit = Some(cfg.profiles.len() - 1);
                        }
                    }
                });
                ui.checkbox(&mut cfg.autoscale_enabled, "Enable auto-scale for profiles");
                ui.horizontal(|ui| {
                    ui.label("Scale hotkey");
                    ui.text_edit_singleline(&mut self.hotkey_buf);
                    if ui.button("Apply").clicked() {
                        cfg.hotkey = self.hotkey_buf.clone();
                        hotkey_apply = true;
                    }
                });
                ui.small("Examples: Ctrl+Alt+S, Ctrl+Shift+F9, Alt+Pause");
            });
        });

        if cfg != before {
            *ctl.config.lock().unwrap() = cfg;
            ctl.save();
            ctl.refresh_live();
            if hotkey_apply { ctl.reload_hotkey(); }
        }
    }
}
