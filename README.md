# FrameForge

An original open-source Windows toolkit for low-latency frame pacing, window capture, scaling, and overlays. It is inspired by the problem space of commercial scaling utilities, but contains no proprietary code, assets, or branding.

## Current MVP
- Rust native desktop UI
- Persistent per-game profiles
- Auto, integer, AMD FSR, NVIDIA NIS, Intel XeSS, and bilinear modes in the profile model
- Scale multiplier, frame limit, VSync, HDR passthrough, cursor capture, and low-latency preferences
- Session telemetry panel
- Windows release build target
- No process injection: capture and presentation are designed around Windows Graphics Capture and DXGI

The UI and profile engine are working now. Capture, shader execution, hotkey registration, and backend implementations are the next engineering layer, not fake toggles pretending to be complete.

## Build
```powershell
cargo run --release
```

## Roadmap
1. Windows Graphics Capture + D3D11 texture import
2. Dedicated fullscreen borderless output window
3. GPU shader pipelines for FSR/NIS/XeSS where redistributable SDK terms allow it
4. Frame-generation providers with an opt-in plugin ABI
5. Raw-input hotkeys, VRR detection, HDR metadata passthrough
6. Benchmark mode, capture graph, per-monitor profiles, shader editor, and plugin SDK

MIT licensed. Contributions welcome.
