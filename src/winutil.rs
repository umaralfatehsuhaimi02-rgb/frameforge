use std::ffi::c_void;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, POINT, RECT, TRUE};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Debug)]
pub struct WinInfo {
    pub hwnd: isize,
    pub title: String,
    pub exe: String,
}

pub fn to_hwnd(v: isize) -> HWND { HWND(v as _) }
pub fn from_hwnd(h: HWND) -> isize { h.0 as isize }

pub fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)); }
    pid
}

pub fn is_own_window(hwnd: HWND) -> bool {
    pid_of(hwnd) == unsafe { GetCurrentProcessId() }
}

pub fn exe_name(hwnd: HWND) -> String {
    unsafe {
        let pid = pid_of(hwnd);
        if pid == 0 { return String::new(); }
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new(); };
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        if ok.is_err() { return String::new(); }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().unwrap_or("").to_lowercase()
    }
}

pub fn window_title(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 { return String::new(); }
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<WinInfo>);
    if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() { return TRUE; }
    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex & WS_EX_TOOLWINDOW.0 != 0 { return TRUE; }
    let title = window_title(hwnd);
    if title.is_empty() || title == "Program Manager" { return TRUE; }
    if is_own_window(hwnd) { return TRUE; }
    let exe = exe_name(hwnd);
    list.push(WinInfo { hwnd: from_hwnd(hwnd), title, exe });
    TRUE
}

pub fn list_windows() -> Vec<WinInfo> {
    let mut list: Vec<WinInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut list as *mut Vec<WinInfo> as *mut c_void as isize));
    }
    list
}

/// Client area of a window in desktop (screen) coordinates.
pub fn client_rect_screen(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut rc = RECT::default();
        GetClientRect(hwnd, &mut rc).ok()?;
        let mut pt = POINT { x: 0, y: 0 };
        if !ClientToScreen(hwnd, &mut pt).as_bool() { return None; }
        Some(RECT { left: pt.x, top: pt.y, right: pt.x + rc.right, bottom: pt.y + rc.bottom })
    }
}
