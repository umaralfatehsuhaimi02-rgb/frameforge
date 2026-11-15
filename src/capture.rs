use anyhow::{anyhow, Result};
use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::{
    IDXGIOutput, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource, DXGI_ERROR_WAIT_TIMEOUT,
    DXGI_OUTDUPL_FRAME_INFO,
};

pub enum Acquired { New, Timeout, Lost }

/// Desktop Duplication of one monitor: frames stay on the GPU (no CPU copy).
pub struct Capture {
    dupl: IDXGIOutputDuplication,
    pub mon: RECT,
}

impl Capture {
    pub fn new(dev: &ID3D11Device, output: &IDXGIOutput) -> Result<Self> {
        unsafe {
            let out1: IDXGIOutput1 = output.cast()?;
            let dupl = out1.DuplicateOutput(dev).map_err(|e| anyhow!("Desktop duplication failed ({e}). Another capture tool may be active."))?;
            let desc = output.GetDesc()?;
            Ok(Self { dupl, mon: desc.DesktopCoordinates })
        }
    }

    /// Waits up to `timeout_ms` for a new desktop image. When one arrives, `on_frame` is called
    /// with the GPU texture while the frame is still held, then the frame is released.
    pub fn acquire(&self, timeout_ms: u32, mut on_frame: impl FnMut(&ID3D11Texture2D)) -> Acquired {
        unsafe {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res: Option<IDXGIResource> = None;
            match self.dupl.AcquireNextFrame(timeout_ms, &mut info, &mut res) {
                Ok(()) => {
                    let mut new = false;
                    // LastPresentTime == 0 means only the mouse pointer changed
                    if info.LastPresentTime != 0 {
                        if let Some(r) = res.take() {
                            if let Ok(t) = r.cast::<ID3D11Texture2D>() {
                                on_frame(&t);
                                new = true;
                            }
                        }
                    }
                    let _ = self.dupl.ReleaseFrame();
                    if new { Acquired::New } else { Acquired::Timeout }
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => Acquired::Timeout,
                Err(_) => Acquired::Lost,
            }
        }
    }
}
