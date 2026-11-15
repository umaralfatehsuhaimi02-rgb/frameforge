use anyhow::{anyhow, Result};
use std::ffi::{c_void, CString};
use windows::core::PCSTR;
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
use windows::Win32::Graphics::Direct3D::{ID3DBlob, ID3DInclude};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;

pub fn blob_bytes(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

fn blob_text(b: &ID3DBlob) -> String {
    String::from_utf8_lossy(blob_bytes(b)).to_string()
}

pub fn compile(src: &str, entry: &str, target: &str) -> Result<ID3DBlob> {
    let entry_c = CString::new(entry)?;
    let target_c = CString::new(target)?;
    let mut code: Option<ID3DBlob> = None;
    let mut errs: Option<ID3DBlob> = None;
    let r = unsafe {
        D3DCompile(
            src.as_ptr() as *const c_void,
            src.len(),
            PCSTR::null(),
            None,
            None::<&ID3DInclude>,
            PCSTR(entry_c.as_ptr() as *const u8),
            PCSTR(target_c.as_ptr() as *const u8),
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut errs),
        )
    };
    if let Err(e) = r {
        let msg = errs.map(|b| blob_text(&b)).unwrap_or_default();
        return Err(anyhow!("shader {entry} failed to compile: {e} {msg}"));
    }
    code.ok_or_else(|| anyhow!("no bytecode for {entry}"))
}

pub struct Tex {
    pub tex: ID3D11Texture2D,
    pub srv: Option<ID3D11ShaderResourceView>,
    pub uav: Option<ID3D11UnorderedAccessView>,
    pub rtv: Option<ID3D11RenderTargetView>,
    pub w: u32,
    pub h: u32,
}

impl Tex {
    pub fn srv(&self) -> &ID3D11ShaderResourceView { self.srv.as_ref().expect("srv") }
    pub fn uav(&self) -> &ID3D11UnorderedAccessView { self.uav.as_ref().expect("uav") }
    pub fn rtv(&self) -> &ID3D11RenderTargetView { self.rtv.as_ref().expect("rtv") }
}

pub fn create_tex(dev: &ID3D11Device, w: u32, h: u32, fmt: DXGI_FORMAT, srv: bool, uav: bool, rtv: bool) -> Result<Tex> {
    let w = w.max(1);
    let h = h.max(1);
    let mut bind = 0u32;
    if srv { bind |= D3D11_BIND_SHADER_RESOURCE.0 as u32; }
    if uav { bind |= D3D11_BIND_UNORDERED_ACCESS.0 as u32; }
    if rtv { bind |= D3D11_BIND_RENDER_TARGET.0 as u32; }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: fmt,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    unsafe {
        let mut t: Option<ID3D11Texture2D> = None;
        dev.CreateTexture2D(&desc, None, Some(&mut t))?;
        let tex = t.ok_or_else(|| anyhow!("texture creation failed"))?;
        let mut out = Tex { tex, srv: None, uav: None, rtv: None, w, h };
        if srv {
            let mut v: Option<ID3D11ShaderResourceView> = None;
            dev.CreateShaderResourceView(&out.tex, None, Some(&mut v))?;
            out.srv = v;
        }
        if uav {
            let mut v: Option<ID3D11UnorderedAccessView> = None;
            dev.CreateUnorderedAccessView(&out.tex, None, Some(&mut v))?;
            out.uav = v;
        }
        if rtv {
            let mut v: Option<ID3D11RenderTargetView> = None;
            dev.CreateRenderTargetView(&out.tex, None, Some(&mut v))?;
            out.rtv = v;
        }
        Ok(out)
    }
}

pub fn create_cbuffer(dev: &ID3D11Device, bytes: u32) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: bytes,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    unsafe {
        let mut b: Option<ID3D11Buffer> = None;
        dev.CreateBuffer(&desc, None, Some(&mut b))?;
        b.ok_or_else(|| anyhow!("buffer creation failed"))
    }
}

pub fn create_sampler(dev: &ID3D11Device, linear: bool) -> Result<ID3D11SamplerState> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: if linear { D3D11_FILTER_MIN_MAG_MIP_LINEAR } else { D3D11_FILTER_MIN_MAG_MIP_POINT },
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        BorderColor: [0.0; 4],
        MinLOD: 0.0,
        MaxLOD: f32::MAX,
    };
    unsafe {
        let mut s: Option<ID3D11SamplerState> = None;
        dev.CreateSamplerState(&desc, Some(&mut s))?;
        s.ok_or_else(|| anyhow!("sampler creation failed"))
    }
}

pub struct Pipelines {
    pub vs: ID3D11VertexShader,
    pub ps_scale: ID3D11PixelShader,
    pub ps_post: ID3D11PixelShader,
    pub cs_luma: ID3D11ComputeShader,
    pub cs_luma_down: ID3D11ComputeShader,
    pub cs_motion: ID3D11ComputeShader,
    pub cs_smooth: ID3D11ComputeShader,
    pub cs_interp: ID3D11ComputeShader,
    pub cs_extrap: ID3D11ComputeShader,
}

const PRESENT_SRC: &str = include_str!("../shaders/present.hlsl");
const FG_SRC: &str = include_str!("../shaders/framegen.hlsl");

fn make_vs(dev: &ID3D11Device, entry: &str) -> Result<ID3D11VertexShader> {
    let b = compile(PRESENT_SRC, entry, "vs_5_0")?;
    let mut o: Option<ID3D11VertexShader> = None;
    unsafe { dev.CreateVertexShader(blob_bytes(&b), None, Some(&mut o))?; }
    o.ok_or_else(|| anyhow!("vs creation failed"))
}

fn make_ps(dev: &ID3D11Device, entry: &str) -> Result<ID3D11PixelShader> {
    let b = compile(PRESENT_SRC, entry, "ps_5_0")?;
    let mut o: Option<ID3D11PixelShader> = None;
    unsafe { dev.CreatePixelShader(blob_bytes(&b), None, Some(&mut o))?; }
    o.ok_or_else(|| anyhow!("ps creation failed"))
}

fn make_cs(dev: &ID3D11Device, entry: &str) -> Result<ID3D11ComputeShader> {
    let b = compile(FG_SRC, entry, "cs_5_0")?;
    let mut o: Option<ID3D11ComputeShader> = None;
    unsafe { dev.CreateComputeShader(blob_bytes(&b), None, Some(&mut o))?; }
    o.ok_or_else(|| anyhow!("cs creation failed"))
}

impl Pipelines {
    pub fn new(dev: &ID3D11Device) -> Result<Self> {
        Ok(Self {
            vs: make_vs(dev, "VSMain")?,
            ps_scale: make_ps(dev, "PSScale")?,
            ps_post: make_ps(dev, "PSPost")?,
            cs_luma: make_cs(dev, "CSLuma")?,
            cs_luma_down: make_cs(dev, "CSLumaDown")?,
            cs_motion: make_cs(dev, "CSMotion")?,
            cs_smooth: make_cs(dev, "CSSmooth")?,
            cs_interp: make_cs(dev, "CSInterp")?,
            cs_extrap: make_cs(dev, "CSExtrap")?,
        })
    }
}
