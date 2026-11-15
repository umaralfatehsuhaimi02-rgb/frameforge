//! Optional mouse remapping so a game window that is scaled up on screen still receives
//! the pointer where the user sees it. Runs on its own thread with a low-level mouse hook.
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::thread::JoinHandle;
use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL};
use windows::Win32::UI::WindowsAndMessaging::*;

static ENABLED: AtomicBool = AtomicBool::new(false);
static SRC: [AtomicI32; 4] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];
static DST: [AtomicI32; 4] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];
static VX: AtomicI32 = AtomicI32::new(0);
static VY: AtomicI32 = AtomicI32::new(0);
static LASTX: AtomicI32 = AtomicI32::new(0);
static LASTY: AtomicI32 = AtomicI32::new(0);

fn load4(a: &[AtomicI32; 4]) -> [i32; 4] {
    [a[0].load(Ordering::Relaxed), a[1].load(Ordering::Relaxed), a[2].load(Ordering::Relaxed), a[3].load(Ordering::Relaxed)]
}

fn store4(a: &[AtomicI32; 4], v: [i32; 4]) {
    for i in 0..4 { a[i].store(v[i], Ordering::Relaxed); }
}

/// Map a point inside the displayed (dst) rect to the matching point in the real window (src) rect.
fn map_point(vx: i32, vy: i32, src: [i32; 4], dst: [i32; 4]) -> (i32, i32) {
    let dw = (dst[2] - dst[0]).max(1) as f32;
    let dh = (dst[3] - dst[1]).max(1) as f32;
    let sw = (src[2] - src[0]) as f32;
    let sh = (src[3] - src[1]) as f32;
    let sx = src[0] + (((vx - dst[0]) as f32) * sw / dw) as i32;
    let sy = src[1] + (((vy - dst[1]) as f32) * sh / dh) as i32;
    (sx.clamp(src[0], src[2] - 1), sy.clamp(src[1], src[3] - 1))
}

/// rects are [left, top, right, bottom] in desktop coordinates.
pub fn configure(enabled: bool, src: [i32; 4], dst: [i32; 4]) {
    let want = enabled && src != dst && src[2] > src[0] && dst[2] > dst[0];
    if !want {
        ENABLED.store(false, Ordering::SeqCst);
        return;
    }
    let changed = !ENABLED.load(Ordering::SeqCst) || load4(&SRC) != src || load4(&DST) != dst;
    if changed {
        ENABLED.store(false, Ordering::SeqCst);
        store4(&SRC, src);
        store4(&DST, dst);
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
            let vx = p.x.clamp(dst[0], dst[2] - 1);
            let vy = p.y.clamp(dst[1], dst[3] - 1);
            let (sx, sy) = map_point(vx, vy, src, dst);
            VX.store(vx, Ordering::SeqCst);
            VY.store(vy, Ordering::SeqCst);
            LASTX.store(sx, Ordering::SeqCst);
            LASTY.store(sy, Ordering::SeqCst);
            let _ = SetCursorPos(sx, sy);
        }
    }
    ENABLED.store(true, Ordering::SeqCst);
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && ENABLED.load(Ordering::Relaxed) && wp.0 as u32 == WM_MOUSEMOVE {
        let ms = &*(lp.0 as *const MSLLHOOKSTRUCT);
        if ms.flags & LLMHF_INJECTED == 0 {
            let src = load4(&SRC);
            let dst = load4(&DST);
            let dx = ms.pt.x - LASTX.load(Ordering::Relaxed);
            let dy = ms.pt.y - LASTY.load(Ordering::Relaxed);
            let vx = (VX.load(Ordering::Relaxed) + dx).clamp(dst[0], dst[2] - 1);
            let vy = (VY.load(Ordering::Relaxed) + dy).clamp(dst[1], dst[3] - 1);
            let (sx, sy) = map_point(vx, vy, src, dst);
            VX.store(vx, Ordering::Relaxed);
            VY.store(vy, Ordering::Relaxed);
            LASTX.store(sx, Ordering::Relaxed);
            LASTY.store(sy, Ordering::Relaxed);
            let _ = SetCursorPos(sx, sy);
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

pub struct CursorHook {
    tid: u32,
    handle: Option<JoinHandle<()>>,
}

pub fn start() -> CursorHook {
    let (tx, rx) = std::sync::mpsc::channel::<u32>();
    let handle = std::thread::spawn(move || unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
        let _ = tx.send(GetCurrentThreadId());
        let hinst = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hinst, 0);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Ok(h) = hook { let _ = UnhookWindowsHookEx(h); }
    });
    let tid = rx.recv().unwrap_or(0);
    CursorHook { tid, handle: Some(handle) }
}

impl CursorHook {
    pub fn stop(mut self) {
        ENABLED.store(false, Ordering::SeqCst);
        unsafe { let _ = PostThreadMessageW(self.tid, WM_QUIT, WPARAM(0), LPARAM(0)); }
        if let Some(h) = self.handle.take() { let _ = h.join(); }
    }
}
