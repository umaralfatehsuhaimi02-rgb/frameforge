use crate::capture::{Acquired, Capture};
use crate::config::{FitMode, FlowQuality, FrameGenMode, Scaler, Settings};
use crate::cursor;
use crate::gfx::*;
use crate::winutil::*;
use anyhow::{anyhow, Result};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{w, Interface};
use windows::Win32::Foundation::{COLORREF, HANDLE, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, HMONITOR, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, WaitForSingleObjectEx, THREAD_PRIORITY_TIME_CRITICAL};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub src_fps: f32,
    pub out_fps: f32,
    pub status: String,
    pub active: bool,
    pub error: Option<String>,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PresentCb {
    src_size: [f32; 2],
    dst_size: [f32; 2],
    dst_rect: [f32; 4],
    mode: u32,
    sharp: f32,
    scanlines: f32,
    saturation: f32,
    contrast: f32,
    osd_on: u32,
    pad: [f32; 2],
    osd0: [u32; 4],
    osd1: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FgCb {
    src_size: [u32; 2],
    luma_size: [u32; 2],
    mv_size: [u32; 2],
    init_size: [u32; 2],
    ds: u32,
    search: u32,
    use_init: u32,
    pad0: u32,
    t: f32,
    thresh: f32,
    pad1: f32,
    pad2: f32,
}

fn div_up(a: u32, b: u32) -> u32 { (a + b - 1) / b }

unsafe fn upd<T: Copy>(ctx: &ID3D11DeviceContext, buf: &ID3D11Buffer, v: &T) {
    ctx.UpdateSubresource(buf, 0, None, v as *const T as *const c_void, 0, 0);
}

struct SrcRes {
    w: u32,
    h: u32,
    fmt: DXGI_FORMAT,
    q: FlowQuality,
    ds: u32,
    color: [Tex; 2],
    luma_f: [Tex; 2],
    luma_c: [Tex; 2],
    mv_c: Tex,
    mv_f: Tex,
    mv2: Tex,
    gen: Tex,
    lf: (u32, u32),
    lc: (u32, u32),
    mc: (u32, u32),
    mf: (u32, u32),
    cur: usize,
    frames: u64,
    fg_frames: u64,
}

impl SrcRes {
    fn new(dev: &ID3D11Device, w: u32, h: u32, fmt: DXGI_FORMAT, q: FlowQuality) -> Result<Self> {
        let ds = q.downscale();
        let lf = (div_up(w, ds), div_up(h, ds));
        let lc = (div_up(lf.0, 2), div_up(lf.1, 2));
        let mc = (div_up(lc.0, 4), div_up(lc.1, 4));
        let mf = (div_up(lf.0, 4), div_up(lf.1, 4));
        let lumaf = DXGI_FORMAT_R32_FLOAT;
        let mvfmt = DXGI_FORMAT_R16G16B16A16_FLOAT;
        Ok(Self {
            w, h, fmt, q, ds,
            color: [create_tex(dev, w, h, fmt, true, false, false)?, create_tex(dev, w, h, fmt, true, false, false)?],
            luma_f: [create_tex(dev, lf.0, lf.1, lumaf, true, true, false)?, create_tex(dev, lf.0, lf.1, lumaf, true, true, false)?],
            luma_c: [create_tex(dev, lc.0, lc.1, lumaf, true, true, false)?, create_tex(dev, lc.0, lc.1, lumaf, true, true, false)?],
            mv_c: create_tex(dev, mc.0, mc.1, mvfmt, true, true, false)?,
            mv_f: create_tex(dev, mf.0, mf.1, mvfmt, true, true, false)?,
            mv2: create_tex(dev, mf.0, mf.1, mvfmt, true, true, false)?,
            gen: create_tex(dev, w, h, DXGI_FORMAT_R8G8B8A8_UNORM, true, true, false)?,
            lf, lc, mc, mf,
            cur: 0,
            frames: 0,
            fg_frames: 0,
        })
    }
}

struct Engine {
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    p: Pipelines,
    lin: ID3D11SamplerState,
    pt: ID3D11SamplerState,
    cb_present: ID3D11Buffer,
    cb_fg: ID3D11Buffer,
    sc: IDXGISwapChain1,
    back_rtv: ID3D11RenderTargetView,
    inter: Tex,
    mon_w: u32,
    mon_h: u32,
    tearing: bool,
    waitable: HANDLE,
    src: Option<SrcRes>,
}

impl Engine {
    unsafe fn ensure_src(&mut self, w: u32, h: u32, fmt: DXGI_FORMAT, q: FlowQuality) -> Result<()> {
        if let Some(r) = &self.src {
            if r.w == w && r.h == h && r.fmt == fmt && r.q == q { return Ok(()); }
        }
        self.src = Some(SrcRes::new(&self.dev, w, h, fmt, q)?);
        Ok(())
    }

    unsafe fn run_cs(
        &self,
        cs: &ID3D11ComputeShader,
        cb: &FgCb,
        srvs: &[(u32, &ID3D11ShaderResourceView)],
        uav: (u32, &ID3D11UnorderedAccessView),
        gw: u32,
        gh: u32,
    ) {
        let ctx = &self.ctx;
        upd(ctx, &self.cb_fg, cb);
        ctx.CSSetShader(cs, None);
        ctx.CSSetConstantBuffers(0, Some(&[Some(self.cb_fg.clone())]));
        ctx.CSSetSamplers(0, Some(&[Some(self.lin.clone())]));
        for (slot, s) in srvs {
            ctx.CSSetShaderResources(*slot, Some(&[Some((*s).clone())]));
        }
        let u: Option<ID3D11UnorderedAccessView> = Some(uav.1.clone());
        ctx.CSSetUnorderedAccessViews(uav.0, 1, Some(&u as *const Option<ID3D11UnorderedAccessView>), None);
        ctx.Dispatch(div_up(gw, 8), div_up(gh, 8), 1);
        let n: Option<ID3D11UnorderedAccessView> = None;
        ctx.CSSetUnorderedAccessViews(uav.0, 1, Some(&n as *const Option<ID3D11UnorderedAccessView>), None);
        for (slot, _) in srvs {
            ctx.CSSetShaderResources(*slot, Some(&[None]));
        }
    }

    /// Copy the captured region into the ring and (optionally) run motion estimation.
    unsafe fn ingest(&mut self, frame: &ID3D11Texture2D, b: &D3D11_BOX, fg: bool) {
        {
            let r = match self.src.as_mut() { Some(r) => r, None => return };
            r.cur ^= 1;
            let cur = r.cur;
            self.ctx.CopySubresourceRegion(&r.color[cur].tex, 0, 0, 0, 0, frame, 0, Some(b as *const D3D11_BOX));
            r.frames += 1;
            if fg { r.fg_frames += 1; } else { r.fg_frames = 0; }
        }
        if !fg { return; }
        let r = self.src.as_ref().unwrap();
        let cur = r.cur;
        let prv = cur ^ 1;
        let p = &self.p;

        let cb = FgCb { src_size: [r.w, r.h], luma_size: [r.lf.0, r.lf.1], ds: r.ds, ..Default::default() };
        self.run_cs(&p.cs_luma, &cb, &[(1, r.color[cur].srv())], (0, r.luma_f[cur].uav()), r.lf.0, r.lf.1);

        let cb = FgCb { src_size: [r.w, r.h], luma_size: [r.lc.0, r.lc.1], ds: r.ds, ..Default::default() };
        self.run_cs(&p.cs_luma_down, &cb, &[(3, r.luma_f[cur].srv())], (0, r.luma_c[cur].uav()), r.lc.0, r.lc.1);

        if r.fg_frames < 2 { return; }

        // coarse motion
        let cb = FgCb {
            src_size: [r.w, r.h],
            luma_size: [r.lc.0, r.lc.1],
            mv_size: [r.mc.0, r.mc.1],
            init_size: [r.mc.0, r.mc.1],
            ds: r.ds, search: 8, use_init: 0,
            ..Default::default()
        };
        self.run_cs(&p.cs_motion, &cb, &[(3, r.luma_c[cur].srv()), (4, r.luma_c[prv].srv())], (1, r.mv_c.uav()), r.mc.0, r.mc.1);

        // fine motion refined around the coarse vector
        let cb = FgCb {
            src_size: [r.w, r.h],
            luma_size: [r.lf.0, r.lf.1],
            mv_size: [r.mf.0, r.mf.1],
            init_size: [r.mc.0, r.mc.1],
            ds: r.ds, search: 3, use_init: 1,
            ..Default::default()
        };
        self.run_cs(&p.cs_motion, &cb, &[(2, r.mv_c.srv()), (3, r.luma_f[cur].srv()), (4, r.luma_f[prv].srv())], (1, r.mv_f.uav()), r.mf.0, r.mf.1);

        // vector median smoothing
        let cb = FgCb { mv_size: [r.mf.0, r.mf.1], ds: r.ds, ..Default::default() };
        self.run_cs(&p.cs_smooth, &cb, &[(2, r.mv_f.srv())], (1, r.mv2.uav()), r.mf.0, r.mf.1);
    }

    unsafe fn gen_frame(&self, r: &SrcRes, extrap: bool, t: f32, thresh: f32) {
        let prv = r.cur ^ 1;
        let cb = FgCb { src_size: [r.w, r.h], mv_size: [r.mf.0, r.mf.1], ds: r.ds, t, thresh, ..Default::default() };
        let cs = if extrap { &self.p.cs_extrap } else { &self.p.cs_interp };
        self.run_cs(
            cs,
            &cb,
            &[(0, r.color[prv].srv()), (1, r.color[r.cur].srv()), (2, r.mv2.srv())],
            (2, r.gen.uav()),
            r.w,
            r.h,
        );
    }

    unsafe fn draw_present(&self, src: &ID3D11ShaderResourceView, sw: u32, sh: u32, dst: [f32; 4], s: &Settings, osd: ([u32; 4], [u32; 4])) {
        let cb = PresentCb {
            src_size: [sw as f32, sh as f32],
            dst_size: [self.mon_w as f32, self.mon_h as f32],
            dst_rect: dst,
            mode: s.scaler.shader_mode(),
            sharp: s.sharpness,
            scanlines: s.scanlines,
            saturation: s.saturation,
            contrast: s.contrast,
            osd_on: s.show_osd as u32,
            pad: [0.0; 2],
            osd0: osd.0,
            osd1: osd.1,
        };
        let ctx = &self.ctx;
        upd(ctx, &self.cb_present, &cb);
        ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        ctx.IASetInputLayout(None);
        let vp = D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: self.mon_w as f32, Height: self.mon_h as f32, MinDepth: 0.0, MaxDepth: 1.0 };
        ctx.RSSetViewports(Some(&[vp]));
        ctx.VSSetShader(&self.p.vs, None);
        ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb_present.clone())]));
        ctx.PSSetSamplers(0, Some(&[Some(self.lin.clone()), Some(self.pt.clone())]));

        ctx.OMSetRenderTargets(Some(&[Some(self.inter.rtv().clone())]), None);
        ctx.PSSetShaderResources(0, Some(&[Some(src.clone())]));
        ctx.PSSetShader(&self.p.ps_scale, None);
        ctx.Draw(3, 0);

        ctx.OMSetRenderTargets(Some(&[Some(self.back_rtv.clone())]), None);
        ctx.PSSetShaderResources(0, Some(&[Some(self.inter.srv().clone())]));
        ctx.PSSetShader(&self.p.ps_post, None);
        ctx.Draw(3, 0);
        ctx.PSSetShaderResources(0, Some(&[None]));
    }

    unsafe fn render(&self, phase: f32, s: &Settings, dst: [f32; 4], osd: ([u32; 4], [u32; 4])) -> Result<()> {
        let r = match self.src.as_ref() { Some(r) if r.frames > 0 => r, _ => return Ok(()) };
        let _ = WaitForSingleObjectEx(self.waitable, 50, true);
        let use_gen = r.fg_frames >= 2
            && match s.frame_gen {
                FrameGenMode::Interpolate => phase < 0.999,
                FrameGenMode::Extrapolate => phase > 0.001,
                FrameGenMode::Off => false,
            };
        let srv = if use_gen {
            self.gen_frame(r, s.frame_gen == FrameGenMode::Extrapolate, phase, s.artifact_guard.max(0.01));
            r.gen.srv().clone()
        } else {
            r.color[r.cur].srv().clone()
        };
        self.draw_present(&srv, r.w, r.h, dst, s, osd);
        let flags = if !s.vsync && self.tearing { DXGI_PRESENT_ALLOW_TEARING } else { DXGI_PRESENT(0) };
        let hr = self.sc.Present(if s.vsync { 1 } else { 0 }, flags);
        if hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET {
            return Err(anyhow!("GPU device was lost"));
        }
        Ok(())
    }
}

fn compute_dst(sw: u32, sh: u32, mw: u32, mh: u32, s: &Settings) -> [f32; 4] {
    let (sw, sh, mw, mh) = (sw as f32, sh as f32, mw as f32, mh as f32);
    let (mut w, mut h) = if s.scaler == Scaler::Integer {
        let k = (mw / sw).min(mh / sh).floor().max(1.0);
        (sw * k, sh * k)
    } else {
        match s.fit {
            FitMode::Fit => { let k = (mw / sw).min(mh / sh); (sw * k, sh * k) }
            FitMode::Stretch => (mw, mh),
            FitMode::Custom => (sw * s.custom_factor, sh * s.custom_factor),
        }
    };
    w = w.min(mw);
    h = h.min(mh);
    let x = ((mw - w) / 2.0).floor();
    let y = ((mh - h) / 2.0).floor();
    [x, y, x + w, y + h]
}

fn osd_codes(src: f32, out: f32) -> ([u32; 4], [u32; 4]) {
    let txt = format!("{}/{}", src.round() as u32, out.round() as u32);
    let mut c = [11u32; 8];
    for (i, ch) in txt.chars().take(8).enumerate() {
        c[i] = match ch { '0'..='9' => ch as u32 - '0' as u32, '/' => 10, _ => 11 };
    }
    ([c[0], c[1], c[2], c[3]], [c[4], c[5], c[6], c[7]])
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(-1),
        WM_MOUSEACTIVATE => LRESULT(3),
        WM_ERASEBKGND => LRESULT(1),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn create_overlay(rc: RECT) -> Result<HWND> {
    let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
    let class = w!("FrameForgeOverlay");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(overlay_proc),
        hInstance: hinst,
        lpszClassName: class,
        ..Default::default()
    };
    RegisterClassExW(&wc);
    let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED;
    let hwnd = CreateWindowExW(
        ex, class, w!("FrameForge Overlay"), WS_POPUP,
        rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top,
        None, None, hinst, None,
    )?;
    SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
    // keep our own overlay out of the capture so we never feed back into ourselves
    let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    Ok(hwnd)
}

unsafe fn find_output(f: &IDXGIFactory1, hmon: HMONITOR) -> Result<(IDXGIAdapter1, IDXGIOutput)> {
    let mut ai = 0;
    while let Ok(a) = f.EnumAdapters1(ai) {
        let mut oi = 0;
        while let Ok(o) = a.EnumOutputs(oi) {
            if let Ok(d) = o.GetDesc() {
                if d.Monitor.0 as isize == hmon.0 as isize { return Ok((a.clone(), o)); }
            }
            oi += 1;
        }
        ai += 1;
    }
    Err(anyhow!("could not find the monitor of the target window"))
}

fn set_status(stats: &Arc<Mutex<Stats>>, s: &str) {
    if let Ok(mut g) = stats.lock() { g.status = s.to_string(); }
}

pub fn run(target: isize, settings: Arc<Mutex<Settings>>, stop: Arc<AtomicBool>, stats: Arc<Mutex<Stats>>) -> Result<()> {
    unsafe { run_inner(target, settings, stop, stats) }
}

unsafe fn run_inner(target: isize, settings: Arc<Mutex<Settings>>, stop: Arc<AtomicBool>, stats: Arc<Mutex<Stats>>) -> Result<()> {
    let target = to_hwnd(target);
    if let Ok(mut g) = stats.lock() { *g = Stats { status: "Starting".into(), active: true, ..Default::default() }; }

    let delay = settings.lock().unwrap().scale_delay_secs;
    for sec in (1..=delay).rev() {
        set_status(&stats, &format!("Scaling in {sec}s..."));
        for _ in 0..10 {
            if stop.load(Ordering::Relaxed) { return Ok(()); }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if !IsWindow(target).as_bool() { return Err(anyhow!("target window is gone")); }
    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);

    let hmon = MonitorFromWindow(target, MONITOR_DEFAULTTONEAREST);
    let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
    let (adapter, output) = find_output(&factory, hmon)?;
    let base: IDXGIAdapter = adapter.cast()?;

    let mut dev_o: Option<ID3D11Device> = None;
    let mut ctx_o: Option<ID3D11DeviceContext> = None;
    D3D11CreateDevice(
        &base, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        Some(&[D3D_FEATURE_LEVEL_11_0]), D3D11_SDK_VERSION, Some(&mut dev_o), None, Some(&mut ctx_o),
    )?;
    let dev = dev_o.ok_or_else(|| anyhow!("no D3D11 device"))?;
    let ctx = ctx_o.ok_or_else(|| anyhow!("no D3D11 context"))?;
    if let Ok(d1) = dev.cast::<IDXGIDevice1>() { let _ = d1.SetMaximumFrameLatency(1); }

    set_status(&stats, "Compiling shaders");
    let p = Pipelines::new(&dev)?;
    let mut cap = Capture::new(&dev, &output)?;
    let mon = cap.mon;
    let mon_w = (mon.right - mon.left) as u32;
    let mon_h = (mon.bottom - mon.top) as u32;
    let overlay = create_overlay(mon)?;

    let factory2: IDXGIFactory2 = base.GetParent()?;
    let mk_desc = |tearing: bool| DXGI_SWAP_CHAIN_DESC1 {
        Width: mon_w,
        Height: mon_h,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        Scaling: DXGI_SCALING_NONE,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        Flags: (DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0
            | if tearing { DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 } else { 0 }) as u32,
    };
    let mut tearing = true;
    let sc: IDXGISwapChain1 = match factory2.CreateSwapChainForHwnd(&dev, overlay, &mk_desc(true), None, None) {
        Ok(s) => s,
        Err(_) => {
            tearing = false;
            factory2.CreateSwapChainForHwnd(&dev, overlay, &mk_desc(false), None, None)?
        }
    };
    let _ = factory2.MakeWindowAssociation(overlay, DXGI_MWA_NO_ALT_ENTER);
    let sc2: IDXGISwapChain2 = sc.cast()?;
    sc2.SetMaximumFrameLatency(1)?;
    let waitable = sc2.GetFrameLatencyWaitableObject();

    let back: ID3D11Texture2D = sc.GetBuffer(0)?;
    let mut rtv_o: Option<ID3D11RenderTargetView> = None;
    dev.CreateRenderTargetView(&back, None, Some(&mut rtv_o))?;
    let back_rtv = rtv_o.ok_or_else(|| anyhow!("no back buffer RTV"))?;

    let mut eng = Engine {
        lin: create_sampler(&dev, true)?,
        pt: create_sampler(&dev, false)?,
        cb_present: create_cbuffer(&dev, std::mem::size_of::<PresentCb>() as u32)?,
        cb_fg: create_cbuffer(&dev, std::mem::size_of::<FgCb>() as u32)?,
        inter: create_tex(&dev, mon_w, mon_h, DXGI_FORMAT_B8G8R8A8_UNORM, true, false, true)?,
        dev, ctx, p, sc, back_rtv, mon_w, mon_h, tearing, waitable, src: None,
    };

    let cursor_hook = cursor::start();
    set_status(&stats, "Scaling");

    let mut result: Result<()> = Ok(());
    let mut visible = true;
    let mut have_src = false;
    let mut clip_box = D3D11_BOX::default();
    let mut dst_rect = [0f32; 4];
    let mut last_geom = Instant::now() - Duration::from_secs(1);
    let mut last_frame_t: Option<Instant> = None;
    let mut interval = 1.0f32 / 60.0;
    let mut pending: VecDeque<(Instant, f32)> = VecDeque::new();
    let (mut src_cnt, mut out_cnt) = (0u32, 0u32);
    let (mut src_fps, mut out_fps) = (0f32, 0f32);
    let mut last_stat = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let s: Settings = *settings.lock().unwrap();

        if last_geom.elapsed() >= Duration::from_millis(40) {
            last_geom = Instant::now();
            if !IsWindow(target).as_bool() || IsIconic(target).as_bool() { break; }
            let fg = GetForegroundWindow();
            let want_visible = fg == target || fg == overlay || !s.hide_when_unfocused;
            if want_visible != visible {
                visible = want_visible;
                if visible {
                    let _ = SetWindowPos(overlay, HWND_TOPMOST, mon.left, mon.top, mon_w as i32, mon_h as i32, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                } else {
                    let _ = ShowWindow(overlay, SW_HIDE);
                }
                pending.clear();
            }
            have_src = false;
            if let Some(rc) = client_rect_screen(target) {
                let l = rc.left.max(mon.left);
                let t = rc.top.max(mon.top);
                let r = rc.right.min(mon.right);
                let b = rc.bottom.min(mon.bottom);
                if r - l >= 16 && b - t >= 16 {
                    have_src = true;
                    clip_box = D3D11_BOX {
                        left: (l - mon.left) as u32, top: (t - mon.top) as u32, front: 0,
                        right: (r - mon.left) as u32, bottom: (b - mon.top) as u32, back: 1,
                    };
                    dst_rect = compute_dst((r - l) as u32, (b - t) as u32, mon_w, mon_h, &s);
                    cursor::configure(
                        s.remap_cursor && visible,
                        [l, t, r, b],
                        [mon.left + dst_rect[0] as i32, mon.top + dst_rect[1] as i32, mon.left + dst_rect[2] as i32, mon.top + dst_rect[3] as i32],
                    );
                }
            }
            if !have_src { cursor::configure(false, [0; 4], [0; 4]); }
        }

        let now = Instant::now();
        let timeout: u32 = if !visible {
            30
        } else {
            match pending.front() {
                Some((due, _)) => {
                    let wait = due.saturating_duration_since(now);
                    if wait > Duration::from_millis(2) { ((wait.as_millis() as u32).saturating_sub(1)).min(30) } else { 0 }
                }
                None => 15,
            }
        };

        let mut got_new = false;
        let acq = cap.acquire(timeout, |frame| {
            if !visible || !have_src { return; }
            let mut d = D3D11_TEXTURE2D_DESC::default();
            frame.GetDesc(&mut d);
            if clip_box.right > d.Width || clip_box.bottom > d.Height { return; }
            let sw = clip_box.right - clip_box.left;
            let sh = clip_box.bottom - clip_box.top;
            if eng.ensure_src(sw, sh, d.Format, s.flow_quality).is_ok() {
                eng.ingest(frame, &clip_box, s.frame_gen != FrameGenMode::Off);
                got_new = true;
            }
        });
        if let Acquired::Lost = acq {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(c) = Capture::new(&eng.dev, &output) { cap = c; }
            continue;
        }

        if got_new {
            let t_new = Instant::now();
            if let Some(prev) = last_frame_t {
                let dt = (t_new - prev).as_secs_f32();
                interval = if dt < 0.25 { (interval * 0.7 + dt * 0.3).clamp(0.002, 0.25) } else { dt.min(0.25) };
            }
            last_frame_t = Some(t_new);
            src_cnt += 1;
            pending.clear();
            let have_prev = eng.src.as_ref().map(|r| r.fg_frames >= 2).unwrap_or(false);
            let n: u32 = if s.frame_gen == FrameGenMode::Off || !have_prev {
                1
            } else if s.adaptive {
                ((s.target_fps as f32 * interval).round() as u32).clamp(1, 8)
            } else {
                s.multiplier.clamp(1, 8)
            };
            for k in 0..n {
                let due = if s.vsync || n == 1 { t_new } else { t_new + Duration::from_secs_f32(interval * k as f32 / n as f32) };
                // interpolation: phases (k+1)/n so the real frame lands on the last slot (lag = interval*(1-1/n))
                // extrapolation: phases k/n so the real frame is shown immediately (no added lag)
                let phase = match s.frame_gen {
                    FrameGenMode::Interpolate => (k + 1) as f32 / n as f32,
                    FrameGenMode::Extrapolate => k as f32 / n as f32,
                    FrameGenMode::Off => 1.0,
                };
                pending.push_back((due, phase));
            }
        }

        if visible && have_src {
            let now = Instant::now();
            let osd = osd_codes(src_fps, out_fps);
            let mut latest: Option<f32> = None;
            while let Some(&(due, ph)) = pending.front() {
                if due > now { break; }
                pending.pop_front();
                if s.vsync {
                    if let Err(e) = eng.render(ph, &s, dst_rect, osd) { result = Err(e); break; }
                    out_cnt += 1;
                } else {
                    latest = Some(ph);
                }
            }
            if result.is_err() { break; }
            if let Some(ph) = latest {
                if let Err(e) = eng.render(ph, &s, dst_rect, osd) { result = Err(e); break; }
                out_cnt += 1;
            }
        }

        if last_stat.elapsed() >= Duration::from_millis(500) {
            let secs = last_stat.elapsed().as_secs_f32();
            src_fps = src_cnt as f32 / secs;
            out_fps = out_cnt as f32 / secs;
            src_cnt = 0;
            out_cnt = 0;
            last_stat = Instant::now();
            if let Ok(mut g) = stats.lock() { g.src_fps = src_fps; g.out_fps = out_fps; g.status = "Scaling".into(); }
        }
    }

    cursor::configure(false, [0; 4], [0; 4]);
    cursor_hook.stop();
    let _ = DestroyWindow(overlay);
    result
}
