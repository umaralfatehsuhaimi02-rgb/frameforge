#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod config;
mod controller;
mod cursor;
mod engine;
mod gfx;
mod gui;
mod winutil;

use eframe::egui;
use windows::Win32::Media::timeBeginPeriod;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

fn main() -> eframe::Result {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = timeBeginPeriod(1);
    }
    let ctl = controller::Controller::new();
    ctl.spawn_hotkey_thread();
    ctl.spawn_autoscale_thread();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FrameForge")
            .with_inner_size([640.0, 760.0])
            .with_min_inner_size([520.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native("FrameForge", options, Box::new(move |_cc| Ok(Box::new(gui::App::new(ctl)))))
}
