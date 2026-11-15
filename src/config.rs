use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Scaler { Nearest, Bilinear, SharpBilinear, Integer, Bicubic, Lanczos, EdgeAdaptive, Anime }

impl Scaler {
    pub const ALL: [Scaler; 8] = [
        Scaler::Nearest, Scaler::Bilinear, Scaler::SharpBilinear, Scaler::Integer,
        Scaler::Bicubic, Scaler::Lanczos, Scaler::EdgeAdaptive, Scaler::Anime,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Scaler::Nearest => "Nearest neighbour",
            Scaler::Bilinear => "Bilinear",
            Scaler::SharpBilinear => "Sharp bilinear (pixel art)",
            Scaler::Integer => "Integer scaling",
            Scaler::Bicubic => "Bicubic (Catmull-Rom)",
            Scaler::Lanczos => "Lanczos 3",
            Scaler::EdgeAdaptive => "Edge-adaptive (FSR-style)",
            Scaler::Anime => "Anime / line-art",
        }
    }
    pub fn shader_mode(self) -> u32 {
        match self {
            Scaler::Nearest | Scaler::Integer => 0,
            Scaler::Bilinear => 1,
            Scaler::SharpBilinear => 2,
            Scaler::Bicubic => 4,
            Scaler::Lanczos => 5,
            Scaler::EdgeAdaptive => 6,
            Scaler::Anime => 7,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum FitMode { Fit, Stretch, Custom }
impl FitMode {
    pub const ALL: [FitMode; 3] = [FitMode::Fit, FitMode::Stretch, FitMode::Custom];
    pub fn label(self) -> &'static str {
        match self { FitMode::Fit => "Fit (keep aspect)", FitMode::Stretch => "Stretch to screen", FitMode::Custom => "Custom factor" }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum FrameGenMode { Off, Interpolate, Extrapolate }
impl FrameGenMode {
    pub const ALL: [FrameGenMode; 3] = [FrameGenMode::Off, FrameGenMode::Interpolate, FrameGenMode::Extrapolate];
    pub fn label(self) -> &'static str {
        match self {
            FrameGenMode::Off => "Off",
            FrameGenMode::Interpolate => "Interpolate (smoothest, low-lag phasing)",
            FrameGenMode::Extrapolate => "Extrapolate (zero added latency)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum FlowQuality { Performance, Balanced, Quality }
impl FlowQuality {
    pub const ALL: [FlowQuality; 3] = [FlowQuality::Performance, FlowQuality::Balanced, FlowQuality::Quality];
    pub fn label(self) -> &'static str {
        match self { FlowQuality::Performance => "Performance", FlowQuality::Balanced => "Balanced", FlowQuality::Quality => "Quality" }
    }
    pub fn downscale(self) -> u32 {
        match self { FlowQuality::Performance => 8, FlowQuality::Balanced => 4, FlowQuality::Quality => 2 }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub scaler: Scaler,
    pub fit: FitMode,
    pub custom_factor: f32,
    pub sharpness: f32,
    pub frame_gen: FrameGenMode,
    pub multiplier: u32,
    pub adaptive: bool,
    pub target_fps: u32,
    pub flow_quality: FlowQuality,
    pub artifact_guard: f32,
    pub vsync: bool,
    pub show_osd: bool,
    pub scanlines: f32,
    pub saturation: f32,
    pub contrast: f32,
    pub remap_cursor: bool,
    pub hide_when_unfocused: bool,
    pub scale_delay_secs: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scaler: Scaler::EdgeAdaptive,
            fit: FitMode::Fit,
            custom_factor: 2.0,
            sharpness: 0.4,
            frame_gen: FrameGenMode::Off,
            multiplier: 2,
            adaptive: false,
            target_fps: 144,
            flow_quality: FlowQuality::Balanced,
            artifact_guard: 0.08,
            vsync: false,
            show_osd: false,
            scanlines: 0.0,
            saturation: 1.0,
            contrast: 1.0,
            remap_cursor: true,
            hide_when_unfocused: true,
            scale_delay_secs: 0,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub exe: String,
    pub auto_scale: bool,
    pub settings: Settings,
}

impl Default for Profile {
    fn default() -> Self {
        Self { name: "New profile".into(), exe: String::new(), auto_scale: false, settings: Settings::default() }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub global: Settings,
    pub profiles: Vec<Profile>,
    pub hotkey: String,
    pub autoscale_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { global: Settings::default(), profiles: Vec::new(), hotkey: "Ctrl+Alt+S".into(), autoscale_enabled: true }
    }
}

impl Config {
    fn path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("FrameForge").join("config.toml")
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        let p = Self::path();
        if let Some(dir) = p.parent() { let _ = std::fs::create_dir_all(dir); }
        if let Ok(s) = toml::to_string_pretty(self) { let _ = std::fs::write(p, s); }
    }
}
