//! Windows output through Direct3D 11: a flip-model swap chain that mpv draws the video into
//! (D3D11VA frames are shown in place, with no copy to memory and back) and a small egui painter
//! for the interface on top. As in VLC's Direct3D 11 output, nothing is created per frame.

use std::collections::HashMap;
use std::ffi::c_void;

use egui::epaint::{ClippedPrimitive, ImageData, Primitive, TextureId, Vertex};
use egui::{TextureFilter, TextureOptions, TextureWrapMode, TexturesDelta};
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{Interface, PCSTR, Result, s};

/// Field order is drop order: the painter's objects before the device.
pub struct Gpu {
    painter: Painter,
    target: Option<(ID3D11Texture2D, ID3D11RenderTargetView)>,
    swap_chain: IDXGISwapChain1,
    context: ID3D11DeviceContext,
    device: ID3D11Device,
}

impl Gpu {
    pub fn new(hwnd: isize, width: u32, height: u32) -> Result<Gpu> {
        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0];
        // Video support lets mpv decode with D3D11VA on this device; a device without it (no
        // video driver) still shows software-decoded video.
        let flags =
            [D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_CREATE_DEVICE_BGRA_SUPPORT];
        let (mut device, mut context, mut result) = (None, None, Ok(()));
        for flag in flags {
            // SAFETY: valid arguments; the out pointers outlive the call.
            result = unsafe {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    flag,
                    Some(&levels),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
            };
            if result.is_ok() {
                break;
            }
        }
        result?;
        let (device, context): (ID3D11Device, ID3D11DeviceContext) = (device.unwrap(), context.unwrap());
        // mpv's decoder thread shares the immediate context with this one, as in VLC.
        // SAFETY: COM calls on a valid device.
        if let Ok(m) = device.cast::<ID3D11Multithread>() {
            let _previous = unsafe { m.SetMultithreadProtected(true) };
        }
        let hwnd = HWND(hwnd as *mut c_void);
        // SAFETY: COM calls on valid objects; `hwnd` is the caller's live window.
        let swap_chain = unsafe {
            let factory: IDXGIFactory2 = device.cast::<IDXGIDevice>()?.GetAdapter()?.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width.max(1),
                Height: height.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                ..Default::default()
            };
            let swap_chain = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)?;
            // Fullscreen is the window's own (borderless); DXGI must not take Alt+Enter.
            let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
            swap_chain
        };
        let painter = Painter::new(&device)?;
        Ok(Gpu { painter, target: None, swap_chain, context, device })
    }

    /// The `ID3D11Device` for mpv.
    pub fn device_ptr(&self) -> *mut c_void {
        self.device.as_raw()
    }

    pub fn max_texture_side(&self) -> usize {
        // SAFETY: plain query on a valid device.
        if unsafe { self.device.GetFeatureLevel() }.0 >= D3D_FEATURE_LEVEL_11_0.0 { 16384 } else { 8192 }
    }

    /// The graphics card's name, for the log.
    pub fn adapter_name(&self) -> String {
        // SAFETY: COM calls on valid objects.
        let desc = unsafe { self.device.cast::<IDXGIDevice>().and_then(|d| d.GetAdapter()).and_then(|a| a.GetDesc()) };
        desc.map(|d| String::from_utf16_lossy(&d.Description).trim_end_matches('\0').to_string()).unwrap_or_default()
    }

    /// The back buffer (`ID3D11Texture2D`) to draw this frame into. With the flip model, buffer 0
    /// is always the current one, so its view is made once per size.
    pub fn back_buffer(&mut self) -> Result<*mut c_void> {
        if self.target.is_none() {
            // SAFETY: COM calls on valid objects.
            let texture: ID3D11Texture2D = unsafe { self.swap_chain.GetBuffer(0)? };
            let mut view = None;
            unsafe { self.device.CreateRenderTargetView(&texture, None, Some(&mut view))? };
            self.target = Some((texture, view.unwrap()));
        }
        Ok(self.target.as_ref().unwrap().0.as_raw())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        // Every reference to the old buffers must be gone first.
        self.target = None;
        // SAFETY: COM call on a valid swap chain whose buffers are no longer referenced.
        let resized = unsafe {
            self.swap_chain.ResizeBuffers(0, width.max(1), height.max(1), DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))
        };
        if let Err(e) = resized {
            log::warn!("swap chain resize failed: {e}");
        }
    }

    pub fn clear(&mut self, rgba: [f32; 4]) {
        if let Some((_, view)) = &self.target {
            // SAFETY: valid view of the current back buffer.
            unsafe { self.context.ClearRenderTargetView(view, &rgba) };
        }
    }

    /// Draws egui's output over the frame. Call after [`Gpu::back_buffer`].
    pub fn paint(
        &mut self,
        size: [u32; 2],
        pixels_per_point: f32,
        primitives: &[ClippedPrimitive],
        delta: &TexturesDelta,
    ) {
        let Some((_, view)) = &self.target else { return };
        if let Err(e) = self.painter.paint(&self.device, &self.context, view, size, pixels_per_point, primitives, delta)
        {
            log::warn!("interface drawing failed: {e}");
        }
    }

    /// Shows the frame at the next vertical blank.
    pub fn present(&self) {
        // SAFETY: COM call on a valid swap chain.
        let hr = unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)) };
        if hr.is_err() {
            log::warn!("present failed: {}", windows::core::Error::from(hr));
        }
    }
}

/// egui's shader: gamma-space vertex colour times gamma-space texel, as in egui_glow.
const SHADER: &str = r"
cbuffer Screen : register(b0) { float2 screen_points; float2 unused; };
struct VsIn { float2 pos : POSITION; float2 uv : TEXCOORD; float4 color : COLOR; };
struct PsIn { float4 pos : SV_POSITION; float2 uv : TEXCOORD; float4 color : COLOR; };
PsIn vs(VsIn i) {
    PsIn o;
    o.pos = float4(2.0 * i.pos.x / screen_points.x - 1.0, 1.0 - 2.0 * i.pos.y / screen_points.y, 0.0, 1.0);
    o.uv = i.uv;
    o.color = i.color;
    return o;
}
Texture2D tex : register(t0);
SamplerState smp : register(s0);
float4 ps(PsIn i) : SV_TARGET { return i.color * tex.Sample(smp, i.uv); }
";

struct Texture {
    texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    sampler: ID3D11SamplerState,
}

struct Painter {
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    screen: ID3D11Buffer,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    /// Dynamic buffers and their capacity in elements; they only grow.
    vertices: Option<(ID3D11Buffer, usize)>,
    indices: Option<(ID3D11Buffer, usize)>,
    textures: HashMap<TextureId, Texture>,
    samplers: HashMap<TextureOptions, ID3D11SamplerState>,
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<Vec<u8>> {
    let (mut code, mut errors) = (None, None);
    // SAFETY: the source pointer and length describe `SHADER`; the out pointers outlive the call.
    let result = unsafe {
        D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            None,
            None,
            None,
            entry,
            target,
            1 << 15, // D3DCOMPILE_OPTIMIZATION_LEVEL3
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    let blob_bytes = |b: &ID3DBlob| unsafe {
        // SAFETY: the blob owns this many bytes at this address.
        std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()).to_vec()
    };
    if let Err(e) = result {
        let message = errors.as_ref().map(|b| String::from_utf8_lossy(&blob_bytes(b)).into_owned()).unwrap_or_default();
        log::error!("interface shader: {message}");
        return Err(e);
    }
    Ok(blob_bytes(code.as_ref().unwrap()))
}

impl Painter {
    fn new(device: &ID3D11Device) -> Result<Painter> {
        let (vs, ps) = (compile(s!("vs"), s!("vs_4_0"))?, compile(s!("ps"), s!("ps_4_0"))?);
        let element = |name: PCSTR, format, offset| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: name,
            SemanticIndex: 0,
            Format: format,
            InputSlot: 0,
            AlignedByteOffset: offset,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        };
        // egui's `Vertex` is `#[repr(C)]`: position and uv as two f32 each, then RGBA bytes.
        let elements = [
            element(s!("POSITION"), DXGI_FORMAT_R32G32_FLOAT, 0),
            element(s!("TEXCOORD"), DXGI_FORMAT_R32G32_FLOAT, 8),
            element(s!("COLOR"), DXGI_FORMAT_R8G8B8A8_UNORM, 16),
        ];
        let mut blend_desc = D3D11_BLEND_DESC::default();
        // Premultiplied alpha, as egui_glow blends.
        blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_INV_DEST_ALPHA,
            DestBlendAlpha: D3D11_BLEND_ONE,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let raster_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            ScissorEnable: true.into(),
            DepthClipEnable: true.into(),
            ..Default::default()
        };
        let screen_desc = D3D11_BUFFER_DESC {
            ByteWidth: 16,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            ..Default::default()
        };
        let (mut vertex_shader, mut pixel_shader, mut layout, mut screen, mut blend, mut raster) =
            (None, None, None, None, None, None);
        // SAFETY: valid descriptions and bytecode; the out pointers outlive the calls.
        unsafe {
            device.CreateVertexShader(&vs, None, Some(&mut vertex_shader))?;
            device.CreatePixelShader(&ps, None, Some(&mut pixel_shader))?;
            device.CreateInputLayout(&elements, &vs, Some(&mut layout))?;
            device.CreateBuffer(&screen_desc, None, Some(&mut screen))?;
            device.CreateBlendState(&blend_desc, Some(&mut blend))?;
            device.CreateRasterizerState(&raster_desc, Some(&mut raster))?;
        }
        Ok(Painter {
            vertex_shader: vertex_shader.unwrap(),
            pixel_shader: pixel_shader.unwrap(),
            layout: layout.unwrap(),
            screen: screen.unwrap(),
            blend: blend.unwrap(),
            raster: raster.unwrap(),
            vertices: None,
            indices: None,
            textures: HashMap::new(),
            samplers: HashMap::new(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn paint(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        view: &ID3D11RenderTargetView,
        [width, height]: [u32; 2],
        pixels_per_point: f32,
        primitives: &[ClippedPrimitive],
        delta: &TexturesDelta,
    ) -> Result<()> {
        for (id, deltas) in &delta.set {
            for d in deltas {
                self.set_texture(device, context, *id, d)?;
            }
        }
        let meshes: Vec<_> = primitives
            .iter()
            .filter_map(|p| match &p.primitive {
                Primitive::Mesh(m) if !m.indices.is_empty() => Some((p.clip_rect, m)),
                _ => None, // VibeLC draws no paint callbacks
            })
            .collect();
        let vertex_count = meshes.iter().map(|(_, m)| m.vertices.len()).sum();
        let index_count = meshes.iter().map(|(_, m)| m.indices.len()).sum();
        if index_count > 0 {
            let vb = grow(device, &mut self.vertices, vertex_count, size_of::<Vertex>(), D3D11_BIND_VERTEX_BUFFER)?;
            let ib = grow(device, &mut self.indices, index_count, 4, D3D11_BIND_INDEX_BUFFER)?;
            // SAFETY: both buffers hold at least the counted elements; each map is written in
            // full range bounds and unmapped before drawing.
            unsafe {
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                context.Map(&vb, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
                let mut out = mapped.pData as *mut Vertex;
                for (_, m) in &meshes {
                    std::ptr::copy_nonoverlapping(m.vertices.as_ptr(), out, m.vertices.len());
                    out = out.add(m.vertices.len());
                }
                context.Unmap(&vb, 0);
                context.Map(&ib, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
                let mut out = mapped.pData as *mut u32;
                for (_, m) in &meshes {
                    std::ptr::copy_nonoverlapping(m.indices.as_ptr(), out, m.indices.len());
                    out = out.add(m.indices.len());
                }
                context.Unmap(&ib, 0);
                self.draw(context, view, &vb, &ib, [width, height], pixels_per_point, &meshes);
            }
        }
        for id in &delta.free {
            self.textures.remove(id);
        }
        Ok(())
    }

    /// Sets the whole pipeline every frame: mpv shares the immediate context and restores nothing.
    ///
    /// # Safety
    /// The buffers hold the meshes' vertices and indices, in order.
    #[allow(clippy::too_many_arguments)]
    unsafe fn draw(
        &self,
        context: &ID3D11DeviceContext,
        view: &ID3D11RenderTargetView,
        vb: &ID3D11Buffer,
        ib: &ID3D11Buffer,
        [width, height]: [u32; 2],
        pixels_per_point: f32,
        meshes: &[(egui::Rect, &egui::Mesh)],
    ) {
        let screen = [width as f32 / pixels_per_point, height as f32 / pixels_per_point, 0.0, 0.0];
        let viewport =
            D3D11_VIEWPORT { Width: width as f32, Height: height as f32, MaxDepth: 1.0, ..Default::default() };
        // SAFETY: COM calls with valid objects and data that outlives each call.
        unsafe {
            context.UpdateSubresource(&self.screen, 0, None, screen.as_ptr().cast(), 0, 0);
            context.OMSetRenderTargets(Some(&[Some(view.clone())]), None);
            context.OMSetBlendState(&self.blend, None, u32::MAX);
            context.OMSetDepthStencilState(None, 0);
            context.RSSetState(&self.raster);
            context.RSSetViewports(Some(&[viewport]));
            context.IASetInputLayout(&self.layout);
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            let (stride, offset) = (size_of::<Vertex>() as u32, 0);
            context.IASetVertexBuffers(0, 1, Some(&Some(vb.clone())), Some(&stride), Some(&offset));
            context.IASetIndexBuffer(ib, DXGI_FORMAT_R32_UINT, 0);
            context.VSSetShader(&self.vertex_shader, None);
            context.VSSetConstantBuffers(0, Some(&[Some(self.screen.clone())]));
            context.PSSetShader(&self.pixel_shader, None);
        }
        let (mut first_index, mut first_vertex) = (0u32, 0i32);
        for (clip, mesh) in meshes {
            let (count, vertices) = (mesh.indices.len() as u32, mesh.vertices.len() as i32);
            let scissor = RECT {
                left: (clip.min.x * pixels_per_point).round().clamp(0.0, width as f32) as i32,
                top: (clip.min.y * pixels_per_point).round().clamp(0.0, height as f32) as i32,
                right: (clip.max.x * pixels_per_point).round().clamp(0.0, width as f32) as i32,
                bottom: (clip.max.y * pixels_per_point).round().clamp(0.0, height as f32) as i32,
            };
            if let Some(t) = self.textures.get(&mesh.texture_id)
                && scissor.right > scissor.left
                && scissor.bottom > scissor.top
            {
                // SAFETY: as above; the index range lies within the mapped data.
                unsafe {
                    context.RSSetScissorRects(Some(&[scissor]));
                    context.PSSetShaderResources(0, Some(&[Some(t.view.clone())]));
                    context.PSSetSamplers(0, Some(&[Some(t.sampler.clone())]));
                    context.DrawIndexed(count, first_index, first_vertex);
                }
            }
            first_index += count;
            first_vertex += vertices;
        }
    }

    fn set_texture(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        id: TextureId,
        delta: &egui::epaint::ImageDelta,
    ) -> Result<()> {
        let ImageData::Color(image) = &delta.image;
        let [w, h] = image.size;
        let (pixels, pitch) = (image.pixels.as_ptr().cast::<c_void>(), (w * 4) as u32);
        if let Some([x, y]) = delta.pos {
            if let Some(t) = self.textures.get(&id) {
                let area = D3D11_BOX {
                    left: x as u32,
                    top: y as u32,
                    front: 0,
                    right: (x + w) as u32,
                    bottom: (y + h) as u32,
                    back: 1,
                };
                // SAFETY: `pixels` holds `h` rows of `pitch` bytes, inside the texture.
                unsafe { context.UpdateSubresource(&t.texture, 0, Some(&area), pixels, pitch, 0) };
            }
            return Ok(());
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w as u32,
            Height: h as u32,
            MipLevels: 1,
            ArraySize: 1,
            // Color32 bytes are sRGB-encoded RGBA, sampled as they are (egui blends in gamma).
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let data = D3D11_SUBRESOURCE_DATA { pSysMem: pixels, SysMemPitch: pitch, SysMemSlicePitch: 0 };
        let (mut texture, mut view) = (None, None);
        // SAFETY: `data` describes the image's pixels, which outlive the calls.
        unsafe {
            device.CreateTexture2D(&desc, Some(&data), Some(&mut texture))?;
            device.CreateShaderResourceView(texture.as_ref().unwrap(), None, Some(&mut view))?;
        }
        let sampler = self.sampler(device, delta.options)?;
        self.textures.insert(id, Texture { texture: texture.unwrap(), view: view.unwrap(), sampler });
        Ok(())
    }

    fn sampler(&mut self, device: &ID3D11Device, options: TextureOptions) -> Result<ID3D11SamplerState> {
        if let Some(s) = self.samplers.get(&options) {
            return Ok(s.clone());
        }
        use TextureFilter::{Linear, Nearest};
        let filter = match (options.minification, options.magnification) {
            (Linear, Linear) => D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            (Nearest, Nearest) => D3D11_FILTER_MIN_MAG_MIP_POINT,
            (Linear, Nearest) => D3D11_FILTER_MIN_LINEAR_MAG_MIP_POINT,
            (Nearest, Linear) => D3D11_FILTER_MIN_POINT_MAG_LINEAR_MIP_POINT,
        };
        let address = match options.wrap_mode {
            TextureWrapMode::ClampToEdge => D3D11_TEXTURE_ADDRESS_CLAMP,
            TextureWrapMode::Repeat => D3D11_TEXTURE_ADDRESS_WRAP,
            TextureWrapMode::MirroredRepeat => D3D11_TEXTURE_ADDRESS_MIRROR,
        };
        let desc = D3D11_SAMPLER_DESC {
            Filter: filter,
            AddressU: address,
            AddressV: address,
            AddressW: address,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut sampler = None;
        // SAFETY: valid description; the out pointer outlives the call.
        unsafe { device.CreateSamplerState(&desc, Some(&mut sampler))? };
        let sampler = sampler.unwrap();
        self.samplers.insert(options, sampler.clone());
        Ok(sampler)
    }
}

/// A dynamic buffer of at least `count` elements of `size` bytes, replaced by a larger one when
/// needed (powers of two, so a growing interface reallocates rarely).
fn grow(
    device: &ID3D11Device,
    slot: &mut Option<(ID3D11Buffer, usize)>,
    count: usize,
    size: usize,
    bind: D3D11_BIND_FLAG,
) -> Result<ID3D11Buffer> {
    if let Some((buffer, capacity)) = slot
        && *capacity >= count
    {
        return Ok(buffer.clone());
    }
    let capacity = count.next_power_of_two().max(1024);
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: (capacity * size) as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        ..Default::default()
    };
    let mut buffer = None;
    // SAFETY: valid description; the out pointer outlives the call.
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer))? };
    let buffer = buffer.unwrap();
    *slot = Some((buffer.clone(), capacity));
    Ok(buffer)
}
