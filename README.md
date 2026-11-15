# FrameForge

Free, open-source real-time upscaler and frame generator for Windows, written in Rust. Original code: nothing is copied from Lossless Scaling or any other product.

## Status: needs a first compile
I wrote this without a Rust or Windows toolchain, so it has **not been compiled or run yet**. Expect a few compile errors on the first `cargo build --release` (Windows API signatures are fussy). Paste them back and they get fixed fast. Treat v0.2 as untested.

## How it works
- **Capture**: DXGI Desktop Duplication, frames stay on the GPU (no CPU copy). The target window's client area is cropped from its monitor.
- **Overlay**: borderless, topmost, click-through window with a flip-model swapchain (waitable object, max frame latency 1, tearing allowed). It is excluded from capture so it never feeds back.
- **Upscalers** (HLSL pixel shaders): nearest, bilinear, sharp bilinear, integer, bicubic (Catmull-Rom), Lanczos 3, edge-adaptive (FSR-style), anime/line-art. Followed by adaptive sharpening (CAS-style), saturation, contrast, scanlines, FPS OSD.
- **Frame generation** (D3D11 compute): luma pyramid, coarse-to-fine block-matching motion estimation, vector-median smoothing, then motion-compensated interpolation or extrapolation with automatic fallback on occlusions and scene cuts. x2 to x8 or adaptive to a target FPS. Three quality levels.
- **Latency**: interpolation presents phases (k+1)/N so the real frame lands on the last slot (added lag = one source frame x (1 - 1/N), half a frame at x2). **Extrapolate mode adds zero latency** but is less clean. No VSync by default, tearing present, time-critical thread, 1 ms timer.
- **Mouse remap**: low-level hook maps the pointer onto the scaled image. Turn off for raw-input shooters.
- Profiles per game (.exe), auto-scale on focus, global hotkey (default Ctrl+Alt+S), start delay. Config in `%APPDATA%\FrameForge\config.toml`.

## Honest limits
- It is not LSFG: no neural network, motion quality is classic block matching.
- "Basically unnoticeable" input delay is real with extrapolation or high native FPS. Interpolation always costs part of a source frame.
- Desktop Duplication means one capture per monitor, SDR output, and it cannot capture protected content.
- Not done yet: HDR output, WGC capture backend, multi-GPU render, xBR/SGSR/NIS ports, per-window cursor overlay.

## Build
```powershell
cargo build --release   # target\release\frameforge.exe
```
CI: move `ci/windows.yml` to `.github/workflows/windows.yml` to get an automatic `.exe` artifact.
