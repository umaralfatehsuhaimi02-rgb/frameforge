use crate::config::{Config, Settings};
use crate::engine::{self, Stats};
use crate::winutil::*;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetMessageW, PostThreadMessageW, MSG, WM_APP, WM_HOTKEY};

struct Running {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    target: isize,
    exe: String,
}

pub struct Controller {
    pub config: Mutex<Config>,
    pub live: Arc<Mutex<Settings>>,
    pub stats: Arc<Mutex<Stats>>,
    run: Mutex<Option<Running>>,
    suppressed: Mutex<Option<isize>>,
    hotkey_tid: AtomicU32,
}

pub fn parse_hotkey(s: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut m = HOT_KEY_MODIFIERS(0);
    let mut vk: Option<u32> = None;
    for part in s.split('+') {
        let p = part.trim().to_ascii_uppercase();
        match p.as_str() {
            "CTRL" | "CONTROL" => m |= MOD_CONTROL,
            "ALT" => m |= MOD_ALT,
            "SHIFT" => m |= MOD_SHIFT,
            "WIN" => m |= MOD_WIN,
            "PAUSE" => vk = Some(0x13),
            "HOME" => vk = Some(0x24),
            "END" => vk = Some(0x23),
            "INSERT" => vk = Some(0x2D),
            "DELETE" => vk = Some(0x2E),
            "PAGEUP" => vk = Some(0x21),
            "PAGEDOWN" => vk = Some(0x22),
            "SCROLLLOCK" => vk = Some(0x91),
            k if k.len() == 1 => {
                let c = k.chars().next().unwrap();
                if c.is_ascii_alphanumeric() { vk = Some(c as u32); }
            }
            k if k.starts_with('F') => {
                if let Ok(n) = k[1..].parse::<u32>() {
                    if (1..=24).contains(&n) { vk = Some(0x70 + n - 1); }
                }
            }
            _ => {}
        }
    }
    vk.map(|k| (m, k))
}

impl Controller {
    pub fn new() -> Arc<Self> {
        let cfg = Config::load();
        let live = cfg.global;
        Arc::new(Self {
            config: Mutex::new(cfg),
            live: Arc::new(Mutex::new(live)),
            stats: Arc::new(Mutex::new(Stats { status: "Idle".into(), ..Default::default() })),
            run: Mutex::new(None),
            suppressed: Mutex::new(None),
            hotkey_tid: AtomicU32::new(0),
        })
    }

    pub fn save(&self) {
        let c = self.config.lock().unwrap().clone();
        c.save();
    }

    pub fn settings_for(&self, exe: &str) -> Settings {
        let cfg = self.config.lock().unwrap();
        if !exe.is_empty() {
            if let Some(p) = cfg.profiles.iter().find(|p| p.exe.eq_ignore_ascii_case(exe)) {
                return p.settings;
            }
        }
        cfg.global
    }

    pub fn refresh_live(&self) {
        let exe = self.run.lock().unwrap().as_ref().map(|r| r.exe.clone()).unwrap_or_default();
        let s = self.settings_for(&exe);
        *self.live.lock().unwrap() = s;
    }

    pub fn is_running(&self) -> bool {
        match self.run.lock().unwrap().as_ref() {
            Some(r) => r.handle.as_ref().map_or(false, |h| !h.is_finished()),
            None => false,
        }
    }

    pub fn start(self: &Arc<Self>, hwnd: isize) {
        self.stop_inner(false);
        let exe = exe_name(to_hwnd(hwnd));
        *self.live.lock().unwrap() = self.settings_for(&exe);
        *self.stats.lock().unwrap() = Stats { status: "Starting".into(), active: true, ..Default::default() };
        let stop = Arc::new(AtomicBool::new(false));
        let (live, stats, st2) = (self.live.clone(), self.stats.clone(), stop.clone());
        let handle = std::thread::Builder::new()
            .name("frameforge-engine".into())
            .spawn(move || {
                let r = engine::run(hwnd, live, st2, stats.clone());
                if let Ok(mut g) = stats.lock() {
                    g.active = false;
                    match r {
                        Ok(()) => g.status = "Idle".into(),
                        Err(e) => { g.error = Some(format!("{e:#}")); g.status = "Stopped (error)".into(); }
                    }
                }
            })
            .ok();
        *self.run.lock().unwrap() = Some(Running { stop, handle, target: hwnd, exe });
    }

    pub fn stop(&self) { self.stop_inner(true); }

    fn stop_inner(&self, suppress: bool) {
        let r = self.run.lock().unwrap().take();
        if let Some(mut r) = r {
            r.stop.store(true, Ordering::SeqCst);
            if suppress { *self.suppressed.lock().unwrap() = Some(r.target); }
            if let Some(h) = r.handle.take() { let _ = h.join(); }
        }
    }

    pub fn toggle_foreground(self: &Arc<Self>) {
        if self.is_running() {
            self.stop();
            return;
        }
        let fg = unsafe { GetForegroundWindow() };
        if from_hwnd(fg) == 0 || is_own_window(fg) { return; }
        self.start(from_hwnd(fg));
    }

    pub fn reload_hotkey(&self) {
        let tid = self.hotkey_tid.load(Ordering::SeqCst);
        if tid != 0 { unsafe { let _ = PostThreadMessageW(tid, WM_APP, WPARAM(0), LPARAM(0)); } }
    }

    pub fn spawn_hotkey_thread(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || unsafe {
            me.hotkey_tid.store(GetCurrentThreadId(), Ordering::SeqCst);
            let register = |me: &Controller| {
                let hk = me.config.lock().unwrap().hotkey.clone();
                if let Some((m, vk)) = parse_hotkey(&hk) { let _ = RegisterHotKey(None, 1, m | MOD_NOREPEAT, vk); }
            };
            register(&me);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                if msg.message == WM_HOTKEY {
                    me.toggle_foreground();
                } else if msg.message == WM_APP {
                    let _ = UnregisterHotKey(None, 1);
                    register(&me);
                }
            }
        });
    }

    /// Starts scaling automatically when a window with an auto-scale profile gets focus.
    pub fn spawn_autoscale_thread(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(600));
            let (enabled, profiles) = {
                let c = me.config.lock().unwrap();
                (c.autoscale_enabled, c.profiles.clone())
            };
            if !enabled || me.is_running() { continue; }
            let fg = unsafe { GetForegroundWindow() };
            let h = from_hwnd(fg);
            if h == 0 || is_own_window(fg) { continue; }
            {
                let mut sup = me.suppressed.lock().unwrap();
                if let Some(s) = *sup {
                    if s == h { continue; }
                    *sup = None;
                }
            }
            let exe = exe_name(fg);
            if exe.is_empty() { continue; }
            if profiles.iter().any(|p| p.auto_scale && p.exe.eq_ignore_ascii_case(&exe)) {
                me.start(h);
            }
        });
    }
}
