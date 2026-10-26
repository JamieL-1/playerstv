//! A flip-model swap chain that shows CPU-drawn BGRA frames, plus optional
//! extra GPU copies that stand in for a game's rendering load.

use anyhow::{Context, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, IDXGISwapChain1, DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

pub struct Presenter {
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain1,
    /// Copied back and forth `load` times per frame.
    scratch: Option<[ID3D11Texture2D; 2]>,
    load: u32,
    sync_interval: u32,
    pitch: u32,
}

impl Presenter {
    pub fn new(hwnd: HWND, width: u32, height: u32, load: u32, sync_interval: u32) -> Result<Self> {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        // SAFETY: the out-pointers are valid locals for the duration of the
        // call; on success both are set.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .context("D3D11CreateDevice")?;
        }
        let device = device.context("D3D11CreateDevice returned no device")?;
        let context = context.context("D3D11CreateDevice returned no context")?;

        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };
        // SAFETY: `hwnd` is a live window owned by this thread, and `desc`
        // outlives the call.
        let swap_chain = unsafe {
            let factory: IDXGIFactory2 =
                CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).context("CreateDXGIFactory2")?;
            factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None).context("CreateSwapChainForHwnd")?
        };

        let scratch = if load > 0 {
            let tex = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                ..Default::default()
            };
            let make = || -> Result<ID3D11Texture2D> {
                let mut t = None;
                // SAFETY: `tex` and the out-pointer are valid for the call.
                unsafe { device.CreateTexture2D(&tex, None, Some(&mut t)).context("CreateTexture2D")? };
                t.context("CreateTexture2D returned no texture")
            };
            Some([make()?, make()?])
        } else {
            None
        };

        Ok(Self { context, swap_chain, scratch, load, sync_interval, pitch: width * 4 })
    }

    /// Uploads one BGRA frame (`pitch * height` bytes) and presents it
    /// after `sync_interval` vertical blanks (0: straight away).
    pub fn present(&self, frame: &[u8]) -> Result<()> {
        // SAFETY: buffer 0 of a flip-discard chain is the current back
        // buffer; `frame` holds a full image at `pitch`, which matches the
        // back buffer's size and format, and outlives UpdateSubresource
        // (it copies before returning).
        unsafe {
            let back: ID3D11Texture2D = self.swap_chain.GetBuffer(0).context("GetBuffer")?;
            self.context.UpdateSubresource(&back, 0, None, frame.as_ptr().cast(), self.pitch, 0);
            if let Some([a, b]) = &self.scratch {
                for n in 0..self.load {
                    let (dst, src) = if n % 2 == 0 { (a, b) } else { (b, a) };
                    self.context.CopyResource(dst, src);
                }
            }
            self.swap_chain.Present(self.sync_interval, DXGI_PRESENT(0)).ok().context("Present")?;
        }
        Ok(())
    }
}
