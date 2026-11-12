//! Direct2D drawing into a premultiplied-alpha DXGI swap chain shown through DirectComposition,
//! so the overlay is per-pixel transparent and GPU accelerated.

use crate::fx::{self, Plow, PALETTE};
use crate::geom::{lerp, V2};
use crate::profiler::Hud;
use crate::world::{ball_radius, FireBox, Kind, Stack, Surface, World, SEG, SPECKLES};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::WaitForSingleObjectEx;
use windows_numerics::{Matrix3x2, Vector2};

/// Drift drawing is cached as GPU-ready meshes (geometry realizations) and rebuilt only when the
/// snow changed: at most every `DRIFT_REBUILD` for slow growth, immediately for urgent changes.
const DRIFT_REBUILD: Duration = Duration::from_millis(300);

/// Drift rebuilds since the last stats report (debug log only).
pub static DRIFT_BUILDS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A drift kept in its own small texture (surface width × drift height). Only segments whose
/// snow changed are redrawn into it; every frame the whole drift is a single textured quad.
struct DriftTex {
    bmp: ID2D1Bitmap1,
    width: usize,
    height: u32,
    seg_rev: Vec<u64>,
    built: Vec<Instant>,
    seen: bool,
}

/// Measures how long the GPU spends on each frame with D3D11 timestamp queries.
/// Results arrive a few frames late; queries are never waited on.
struct GpuTimer {
    ctx: ID3D11DeviceContext,
    sets: Vec<(ID3D11Query, ID3D11Query, ID3D11Query)>,
    next: Cell<usize>,
    pending: RefCell<std::collections::VecDeque<usize>>,
}

impl GpuTimer {
    fn new(d3d: &ID3D11Device) -> Result<GpuTimer> {
        let make = |q: D3D11_QUERY| -> Result<ID3D11Query> {
            let mut out = None;
            unsafe { d3d.CreateQuery(&D3D11_QUERY_DESC { Query: q, MiscFlags: 0 }, Some(&mut out))? };
            out.ok_or_else(|| Error::from(E_FAIL))
        };
        let sets = (0..6)
            .map(|_| {
                Ok((make(D3D11_QUERY_TIMESTAMP_DISJOINT)?, make(D3D11_QUERY_TIMESTAMP)?, make(D3D11_QUERY_TIMESTAMP)?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(GpuTimer {
            ctx: unsafe { d3d.GetImmediateContext()? },
            sets,
            next: Cell::new(0),
            pending: RefCell::new(Default::default()),
        })
    }

    fn begin(&self) {
        let i = self.next.get();
        let mut pending = self.pending.borrow_mut();
        if pending.contains(&i) {
            // the GPU is far behind; give up on that old sample
            pending.retain(|&k| k != i);
        }
        unsafe {
            self.ctx.Begin(&self.sets[i].0);
            self.ctx.End(&self.sets[i].1);
        }
    }

    fn end(&self) {
        let i = self.next.get();
        unsafe {
            self.ctx.End(&self.sets[i].2);
            self.ctx.End(&self.sets[i].0);
        }
        self.pending.borrow_mut().push_back(i);
        self.next.set((i + 1) % self.sets.len());
    }

    /// GPU time of the oldest finished frame, ms.
    fn poll(&self) -> Option<f32> {
        let mut pending = self.pending.borrow_mut();
        let &i = pending.front()?;
        let (dq, q0, q1) = &self.sets[i];
        let flag = D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32;
        let mut dj = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT { Frequency: 0, Disjoint: BOOL(1) };
        let (mut t0, mut t1) = (0u64, 0u64);
        unsafe {
            // S_FALSE ("not ready yet") also maps to Ok, so the zeroed outputs tell us
            let _ = self.ctx.GetData(dq, Some(&mut dj as *mut _ as *mut _), std::mem::size_of_val(&dj) as u32, flag);
            if dj.Frequency == 0 {
                return None;
            }
            let _ = self.ctx.GetData(q0, Some(&mut t0 as *mut _ as *mut _), 8, flag);
            let _ = self.ctx.GetData(q1, Some(&mut t1 as *mut _ as *mut _), 8, flag);
        }
        if t0 == 0 || t1 == 0 {
            return None;
        }
        pending.pop_front();
        (!dj.Disjoint.as_bool() && t1 >= t0).then(|| ((t1 - t0) as f64 / dj.Frequency as f64 * 1000.0) as f32)
    }
}

/// A sprite from assets/sprites, embedded in the exe.
struct Img {
    bmp: ID2D1Bitmap1,
    w: f32,
    h: f32,
}

/// Decodes an embedded PNG into a premultiplied BGRA Direct2D bitmap.
fn load_png(dc: &ID2D1DeviceContext3, bytes: &[u8]) -> Result<Img> {
    let bad = |_| Error::from(E_FAIL);
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(bad)?;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or_else(|| Error::from(E_FAIL))?];
    let info = reader.next_frame(&mut buf).map_err(bad)?;
    let (w, h) = (info.width as usize, info.height as usize);
    let ch = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        _ => 1,
    };
    let mut px = vec![0u8; w * h * 4];
    for i in 0..w * h {
        let s = &buf[i * ch..i * ch + ch];
        let (r, g, b, a) = match ch {
            4 => (s[0], s[1], s[2], s[3]),
            3 => (s[0], s[1], s[2], 255),
            2 => (s[0], s[0], s[0], s[1]),
            _ => (s[0], s[0], s[0], 255),
        };
        let m = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
        px[i * 4] = m(b);
        px[i * 4 + 1] = m(g);
        px[i * 4 + 2] = m(r);
        px[i * 4 + 3] = a;
    }
    let bmp = unsafe {
        dc.CreateBitmap(
            D2D_SIZE_U { width: w as u32, height: h as u32 },
            Some(px.as_ptr() as *const _),
            (w * 4) as u32,
            &sprite_props(),
        )?
    };
    Ok(Img { bmp, w: w as f32, h: h as f32 })
}

/// An axis-aligned area in render coordinates.
#[derive(Clone, Copy, PartialEq)]
struct Area {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
}

impl Area {
    fn rect(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.l, top: self.t, right: self.r, bottom: self.b }
    }
}

/// `src` minus every occluder, as a few disjoint rectangles.
fn subtract(src: Area, occluders: &[Area]) -> Vec<Area> {
    let mut parts = vec![src];
    for o in occluders {
        let mut next = Vec::with_capacity(parts.len() + 3);
        for p in parts {
            if o.r <= p.l || o.l >= p.r || o.b <= p.t || o.t >= p.b {
                next.push(p);
                continue;
            }
            if o.t > p.t {
                next.push(Area { b: o.t, ..p });
            }
            if o.b < p.b {
                next.push(Area { t: o.b, ..p });
            }
            let (t, b) = (p.t.max(o.t), p.b.min(o.b));
            if o.l > p.l {
                next.push(Area { l: p.l, t, r: o.l, b });
            }
            if o.r < p.r {
                next.push(Area { l: o.r, t, r: p.r, b });
            }
        }
        parts = next;
        if parts.is_empty() {
            break;
        }
    }
    parts
}

/// Sprites collected for one `DrawSpriteBatch` call.
#[derive(Default)]
struct Sprites {
    rects: Vec<D2D_RECT_F>,
    colors: Vec<D2D1_COLOR_F>,
}

impl Sprites {
    fn push(&mut self, c: V2, r: f32, a: f32) {
        self.rects.push(D2D_RECT_F { left: c.x - r, top: c.y - r, right: c.x + r, bottom: c.y + r });
        self.colors.push(D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a });
    }
}

pub struct Renderer {
    _d3d: ID3D11Device,
    swap: IDXGISwapChain1,
    waitable: HANDLE,
    factory: ID2D1Factory1,
    dc: ID2D1DeviceContext3,
    gpu_timer: Option<GpuTimer>,
    adapter: Option<IDXGIAdapter3>,
    pub adapter_name: String,
    hud_fmt: IDWriteTextFormat,
    batches: Vec<ID2D1SpriteBatch>,
    next_batch: Cell<usize>,
    /// last frame: [scene recording ms, EndDraw ms, drifts ms] (debug log)
    pub timing: Cell<[f32; 3]>,
    drift_cache: RefCell<HashMap<u64, DriftTex>>,
    target_bmp: ID2D1Bitmap1,
    _dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    origin: V2,
    flake: ID2D1Bitmap1,
    ball: ID2D1Bitmap1,
    plow_img: Img,
    box_img: Img,
    hats: Vec<Img>,
    scarves: Vec<Img>,
    arm_img: Img,
    nose_img: Img,
    faces: Vec<Img>,
    glow: Vec<ID2D1Bitmap1>,
    pile: ID2D1SolidColorBrush,
    edge: ID2D1SolidColorBrush,
    speck: ID2D1SolidColorBrush,
    solid: ID2D1SolidColorBrush,
    label_fmt: IDWriteTextFormat,
    banner_fmt: IDWriteTextFormat,
    sub_fmt: IDWriteTextFormat,
    /// vblanks per frame: keeps ~60 fps on 120/144/180 Hz monitors
    pub sync: u32,
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.waitable);
        }
    }
}

fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn pt(p: V2) -> Vector2 {
    Vector2 { X: p.x, Y: p.y }
}

fn sprite_props() -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        ..Default::default()
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn put(v: &mut [u8], i: usize, r: f32, g: f32, b: f32, a: f32) {
    let a = a.clamp(0.0, 1.0);
    v[i] = (b.clamp(0.0, 1.0) * a * 255.0) as u8;
    v[i + 1] = (g.clamp(0.0, 1.0) * a * 255.0) as u8;
    v[i + 2] = (r.clamp(0.0, 1.0) * a * 255.0) as u8;
    v[i + 3] = (a * 255.0) as u8;
}

/// Soft round flake: bright core, slightly bluish rim so it stays visible on white.
fn flake_pixels(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) / 2.0;
    for y in 0..n {
        for x in 0..n {
            let d = (((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt() / (n as f32 / 2.0)).min(1.0);
            let a = (1.0 - smoothstep(0.25, 1.0, d)).powf(1.3);
            let rim = smoothstep(0.3, 0.8, d);
            put(&mut v, (y * n + x) * 4, lerp(1.0, 0.80, rim), lerp(1.0, 0.86, rim), lerp(1.0, 0.94, rim), a);
        }
    }
    v
}

/// Glowing spark of one colour with a white-hot core.
fn glow_pixels(n: usize, (r, g, b): (f32, f32, f32)) -> Vec<u8> {
    let mut v = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) / 2.0;
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt() / (n as f32 / 2.0);
            let a = (-d * d * 5.0).exp() * (1.0 - smoothstep(0.85, 1.0, d));
            let core = (-d * d * 30.0).exp();
            put(&mut v, (y * n + x) * 4, lerp(r, 1.0, core), lerp(g, 1.0, core), lerp(b, 1.0, core), a);
        }
    }
    v
}

/// Shaded snow sphere lit from the upper left, with a little grain.
fn ball_pixels(n: usize) -> Vec<u8> {
    let mut rng = fastrand::Rng::with_seed(7);
    let mut v = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) / 2.0;
    let rad = n as f32 / 2.0 - 1.0;
    let l = {
        let (x, y, z) = (-0.45f32, -0.6f32, 0.66f32);
        let k = (x * x + y * y + z * z).sqrt();
        (x / k, y / k, z / k)
    };
    for y in 0..n {
        for x in 0..n {
            let dx = (x as f32 - c) / rad;
            let dy = (y as f32 - c) / rad;
            let d = (dx * dx + dy * dy).sqrt();
            let a = ((1.0 - d) * rad).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let nz = (1.0 - (d * d).min(1.0)).sqrt();
            let lam = (dx * l.0 + dy * l.1 + nz * l.2).max(0.0);
            let shade = lam.powf(0.7);
            let spec = lam.powf(30.0) * 0.25;
            let grain = (rng.f32() - 0.5) * 0.05;
            let rim = smoothstep(0.82, 1.0, d) * 0.35;
            let r = lerp(0.70, 1.0, shade) + spec + grain - rim;
            let g = lerp(0.76, 1.0, shade) + spec + grain - rim * 0.9;
            let b = lerp(0.88, 1.0, shade) + spec + grain - rim * 0.6;
            put(&mut v, (y * n + x) * 4, r, g, b, a);
        }
    }
    v
}

impl Renderer {
    pub fn new(hwnd: HWND, origin: V2, w: u32, h: u32, scale: f32) -> Result<Renderer> {
        unsafe {
            let mut d3d = None;
            let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
            if D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )
            .is_err()
            {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_WARP,
                    HMODULE::default(),
                    flags,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut d3d),
                    None,
                    None,
                )?;
            }
            let d3d = d3d.ok_or_else(|| Error::from(E_FAIL))?;
            let dxgi: IDXGIDevice = d3d.cast()?;
            let fac: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w,
                Height: h,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Scaling: DXGI_SCALING_STRETCH,
                Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
                ..Default::default()
            };
            let swap = fac.CreateSwapChainForComposition(&dxgi, &desc, None)?;
            let adapter: Option<IDXGIAdapter3> = dxgi.GetAdapter().ok().and_then(|a| a.cast().ok());
            let adapter_name = adapter
                .as_ref()
                .and_then(|a| a.GetDesc().ok())
                .map(|d| String::from_utf16_lossy(&d.Description).trim_end_matches('\0').to_string())
                .unwrap_or_default();
            let gpu_timer = GpuTimer::new(&d3d).ok();
            // one frame in flight, so window positions are sampled as late as possible
            let swap2: IDXGISwapChain2 = swap.cast()?;
            swap2.SetMaximumFrameLatency(1)?;
            let waitable = swap2.GetFrameLatencyWaitableObject();

            let opts = D2D1_FACTORY_OPTIONS::default();
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&opts))?;
            let dev = factory.CreateDevice(&dxgi)?;
            let dc: ID2D1DeviceContext3 = dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?.cast()?;
            let surf: IDXGISurface = swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..sprite_props()
            };
            let target_bmp = dc.CreateBitmapFromDxgiSurface(&surf, Some(&props))?;
            dc.SetTarget(&target_bmp);
            let target_bmp: ID2D1Bitmap1 = target_bmp;
            dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;

            let mk = |n: usize, px: Vec<u8>| -> Result<ID2D1Bitmap1> {
                dc.CreateBitmap(
                    D2D_SIZE_U { width: n as u32, height: n as u32 },
                    Some(px.as_ptr() as *const _),
                    (n * 4) as u32,
                    &sprite_props(),
                )
            };
            let flake = mk(32, flake_pixels(32))?;
            let ball = mk(256, ball_pixels(256))?;
            let plow_img = load_png(&dc, include_bytes!("../assets/sprites/plow.png"))?;
            let box_img = load_png(&dc, include_bytes!("../assets/sprites/box.png"))?;
            // snowman accessories: every snowman gets a random combination
            let hats = vec![
                load_png(&dc, include_bytes!("../assets/sprites/hat.png"))?,
                load_png(&dc, include_bytes!("../assets/sprites/hat_santa.png"))?,
                load_png(&dc, include_bytes!("../assets/sprites/hat_beanie.png"))?,
                load_png(&dc, include_bytes!("../assets/sprites/hat_bucket.png"))?,
            ];
            let scarves = vec![
                load_png(&dc, include_bytes!("../assets/sprites/scarf.png"))?,
                load_png(&dc, include_bytes!("../assets/sprites/scarf_blue.png"))?,
                load_png(&dc, include_bytes!("../assets/sprites/scarf_green.png"))?,
            ];
            let arm_img = load_png(&dc, include_bytes!("../assets/sprites/arm.png"))?;
            let nose_img = load_png(&dc, include_bytes!("../assets/sprites/nose.png"))?;
            let faces = vec![
                load_png(&dc, include_bytes!("../assets/sprites/face.png"))?,
                load_png(&dc, include_bytes!("../assets/sprites/face_wink.png"))?,
                load_png(&dc, include_bytes!("../assets/sprites/face_grin.png"))?,
            ];
            let glow = PALETTE.iter().map(|c| mk(32, glow_pixels(32, *c))).collect::<Result<Vec<_>>>()?;
            let pile = dc.CreateSolidColorBrush(&color(0.975, 0.985, 1.0, 1.0), None)?;
            let edge = dc.CreateSolidColorBrush(&color(0.55, 0.64, 0.78, 0.75), None)?;
            let speck = dc.CreateSolidColorBrush(&color(0.55, 0.62, 0.74, 0.3), None)?;
            let solid = dc.CreateSolidColorBrush(&color(1.0, 1.0, 1.0, 1.0), None)?;

            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let fmt = |size: f32, weight: DWRITE_FONT_WEIGHT| -> Result<IDWriteTextFormat> {
                let f = dw.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("ru-ru"),
                )?;
                f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                Ok(f)
            };
            let label_fmt = fmt(12.0 * scale, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            let banner_fmt = fmt(120.0 * scale, DWRITE_FONT_WEIGHT_BOLD)?;
            let sub_fmt = fmt(56.0 * scale, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            let hud_fmt = dw.CreateTextFormat(
                w!("Consolas"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                13.0 * scale,
                w!("en-us"),
            )?;
            hud_fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;

            Ok(Renderer {
                _d3d: d3d,
                swap,
                waitable,
                factory,
                batches: (0..12).map(|_| dc.CreateSpriteBatch()).collect::<Result<Vec<_>>>()?,
                next_batch: Cell::new(0),
                timing: Cell::new([0.0; 3]),
                drift_cache: RefCell::new(HashMap::new()),
                target_bmp,
                dc,
                gpu_timer,
                adapter,
                adapter_name,
                hud_fmt,
                _dcomp: dcomp,
                _target: target,
                _visual: visual,
                origin,
                flake,
                ball,
                plow_img,
                box_img,
                hats,
                scarves,
                arm_img,
                nose_img,
                faces,
                glow,
                pile,
                edge,
                speck,
                solid,
                label_fmt,
                banner_fmt,
                sub_fmt,
                sync: 1,
            })
        }
    }

    /// Blocks until the swap chain can take the next frame.
    pub fn wait(&self) {
        unsafe {
            WaitForSingleObjectEx(self.waitable, 100, true);
        }
    }

    pub fn draw(&mut self, w: &World, hud: Option<&Hud>) -> Result<()> {
        unsafe {
            self.next_batch.set(0);
            if let Some(t) = &self.gpu_timer {
                t.begin();
            }
            let td = Instant::now();
            if w.active {
                self.update_drift_textures(w)?;
            }
            let drift_ms = td.elapsed().as_secs_f32() * 1000.0;
            self.dc.BeginDraw();
            self.dc.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
            let t0 = Instant::now() - Duration::from_secs_f32(drift_ms / 1000.0);
            if w.active {
                self.draw_world(w)?;
                if let Some(h) = hud {
                    self.draw_hud(w, h);
                }
            }
            let t1 = Instant::now();
            let r = self.dc.EndDraw(None, None);
            if let Some(t) = &self.gpu_timer {
                t.end();
            }
            let t2 = Instant::now();
            self.timing.set([(t1 - t0).as_secs_f32() * 1000.0, (t2 - t1).as_secs_f32() * 1000.0, self.timing.get()[2]]);
            r
        }
    }

    /// GPU time of a recent frame, ms (arrives a few frames late).
    pub fn poll_gpu(&self) -> Option<f32> {
        self.gpu_timer.as_ref()?.poll()
    }

    /// Video memory this process uses on the GPU, MB.
    pub fn vram_mb(&self) -> f32 {
        let Some(a) = &self.adapter else { return 0.0 };
        let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        unsafe {
            if a.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info).is_err() {
                return 0.0;
            }
        }
        info.CurrentUsage as f32 / (1024.0 * 1024.0)
    }

    fn draw_hud(&self, w: &World, h: &Hud) {
        let s = w.scale;
        let mon = w.monitors.iter().find(|m| m.rect.l == 0 && m.rect.t == 0).or(w.monitors.first());
        let Some(mon) = mon else { return };
        let (x, y) = (mon.work.l as f32 + 16.0 * s, mon.work.t as f32 + 16.0 * s);
        let line_h = 17.0 * s;
        let (gw, gh) = (480.0 * s, 64.0 * s);
        let width = 600.0 * s;
        let height = 16.0 * s + (h.lines.len() + 1) as f32 * line_h + gh + 16.0 * s;
        self.fill_rect(x, y, x + width, y + height, 8.0 * s, color(0.03, 0.05, 0.1, 0.82));
        let o = self.origin;
        for (i, line) in h.lines.iter().enumerate() {
            let wide: Vec<u16> = line.encode_utf16().collect();
            let ly = y + 8.0 * s + i as f32 * line_h;
            let rect = D2D_RECT_F {
                left: x + 12.0 * s - o.x,
                top: ly - o.y,
                right: x + width - o.x,
                bottom: ly + line_h - o.y,
            };
            let col = if i == 0 { color(1.0, 0.85, 0.4, 1.0) } else { color(0.88, 0.93, 1.0, 1.0) };
            unsafe {
                self.solid.SetColor(&col);
                self.dc.DrawText(
                    &wide,
                    &self.hud_fmt,
                    &rect,
                    &self.solid,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
        }
        // frame-time graph: CPU (blue) and GPU (orange); the scale follows the recent peak
        let gx = x + 12.0 * s;
        let gy = y + 8.0 * s + h.lines.len() as f32 * line_h + 6.0 * s + gh;
        let peak = h.cpu.iter().chain(&h.gpu).fold(0.0f32, |m, v| m.max(*v));
        let scale_ms = [1.0f32, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0].into_iter().find(|v| *v >= peak).unwrap_or(100.0);
        let bw = gw / 240.0;
        let bars = |vals: &[f32], l: f32, r: f32, col: D2D1_COLOR_F| {
            let n = vals.len().min(240);
            for (i, v) in vals[vals.len() - n..].iter().enumerate() {
                let bh = (v / scale_ms).min(1.0) * gh;
                let bx = gx + (240 - n + i) as f32 * bw;
                self.fill_rect(bx + bw * l, gy - bh, bx + bw * r, gy, 0.0, col);
            }
        };
        bars(&h.cpu, 0.0, 0.9, color(0.35, 0.65, 1.0, 0.9));
        bars(&h.gpu, 0.25, 0.65, color(1.0, 0.6, 0.2, 0.9));
        self.line(V2(gx, gy - gh), V2(gx + gw, gy - gh), 1.0, color(1.0, 1.0, 1.0, 0.25));
        self.line(V2(gx, gy), V2(gx + gw, gy), 1.0, color(1.0, 1.0, 1.0, 0.5));
        let legend: Vec<u16> =
            format!("blue: CPU frame, orange: GPU frame, top line = {scale_ms:.0} ms (budget {:.1} ms)", h.budget)
                .encode_utf16()
                .collect();
        let rect = D2D_RECT_F {
            left: gx - o.x,
            top: gy + 2.0 * s - o.y,
            right: x + width - o.x,
            bottom: gy + line_h + 2.0 * s - o.y,
        };
        unsafe {
            self.solid.SetColor(&color(0.7, 0.75, 0.85, 1.0));
            self.dc.DrawText(
                &legend,
                &self.hud_fmt,
                &rect,
                &self.solid,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    pub fn present(&self) -> Result<()> {
        unsafe { self.swap.Present(self.sync, DXGI_PRESENT(0)).ok() }
    }

    // ---------------------------------------------------------------- primitives

    fn sprite(&self, bmp: &ID2D1Bitmap1, c: V2, r: f32, opacity: f32) {
        let c = c - self.origin;
        let rect = D2D_RECT_F { left: c.x - r, top: c.y - r, right: c.x + r, bottom: c.y + r };
        unsafe {
            self.dc.DrawBitmap(bmp, Some(&rect), opacity, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
        }
    }

    /// Window rectangles in render coordinates.
    fn areas(&self, rects: &[crate::geom::RectI]) -> Vec<Area> {
        let o = self.origin;
        rects
            .iter()
            .map(|r| Area { l: r.l as f32 - o.x, t: r.t as f32 - o.y, r: r.r as f32 - o.x, b: r.b as f32 - o.y })
            .collect()
    }

    /// Runs `draw` so that it only paints inside `area` and never over the windows in `front`.
    fn occluded(&self, area: Area, front: &[Area], mut draw: impl FnMut() -> Result<()>) -> Result<()> {
        let parts = subtract(area, front);
        if parts.len() == 1 && parts[0] == area && front.is_empty() {
            return draw();
        }
        for p in parts {
            unsafe { self.dc.PushAxisAlignedClip(&p.rect(), D2D1_ANTIALIAS_MODE_ALIASED) };
            let r = draw();
            unsafe { self.dc.PopAxisAlignedClip() };
            r?;
        }
        Ok(())
    }

    /// Draws many copies of one bitmap in a single call (positions in screen coordinates).
    fn flush(&self, sp: &mut Sprites, bmp: &ID2D1Bitmap1) {
        let n = sp.rects.len();
        if n == 0 {
            return;
        }
        let i = self.next_batch.get();
        let Some(batch) = self.batches.get(i) else {
            // out of batches this frame: fall back to plain bitmap draws
            for (r, c) in sp.rects.iter().zip(&sp.colors) {
                let c2 = V2((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                self.sprite(bmp, c2, (r.right - r.left) / 2.0, c.a);
            }
            sp.rects.clear();
            sp.colors.clear();
            return;
        };
        self.next_batch.set(i + 1);
        let o = self.origin;
        for r in &mut sp.rects {
            r.left -= o.x;
            r.right -= o.x;
            r.top -= o.y;
            r.bottom -= o.y;
        }
        unsafe {
            batch.Clear();
            let ok = batch
                .AddSprites(
                    n as u32,
                    sp.rects.as_ptr(),
                    None,
                    Some(sp.colors.as_ptr()),
                    None,
                    std::mem::size_of::<D2D_RECT_F>() as u32,
                    0,
                    std::mem::size_of::<D2D1_COLOR_F>() as u32,
                    0,
                )
                .is_ok();
            if ok {
                // sprite batches require aliased mode
                self.dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
                self.dc.DrawSpriteBatch(
                    batch,
                    0,
                    n as u32,
                    bmp,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    D2D1_SPRITE_OPTIONS_NONE,
                );
                self.dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            }
        }
        sp.rects.clear();
        sp.colors.clear();
    }

    /// Draws `img` into a screen rectangle; `flip` mirrors it horizontally.
    fn image(&self, img: &Img, l: f32, t: f32, w: f32, h: f32, flip: bool) {
        let o = self.origin;
        let rect = D2D_RECT_F { left: l - o.x, top: t - o.y, right: l + w - o.x, bottom: t + h - o.y };
        unsafe {
            if flip {
                let cx = l - o.x + w / 2.0;
                self.dc.SetTransform(&Matrix3x2 { M11: -1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 2.0 * cx, M32: 0.0 });
            }
            self.dc.DrawBitmap(&img.bmp, Some(&rect), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
            if flip {
                self.dc.SetTransform(&Matrix3x2::identity());
            }
        }
    }

    fn fill_rect(&self, l: f32, t: f32, r: f32, b: f32, radius: f32, c: D2D1_COLOR_F) {
        let o = self.origin;
        let (l, r) = if l < r { (l, r) } else { (r, l) };
        let rect = D2D_RECT_F { left: l - o.x, top: t - o.y, right: r - o.x, bottom: b - o.y };
        unsafe {
            self.solid.SetColor(&c);
            if radius > 0.0 {
                self.dc
                    .FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect, radiusX: radius, radiusY: radius }, &self.solid);
            } else {
                self.dc.FillRectangle(&rect, &self.solid);
            }
        }
    }

    fn fill_circle(&self, c: V2, r: f32, col: D2D1_COLOR_F) {
        unsafe {
            self.solid.SetColor(&col);
            let e = D2D1_ELLIPSE { point: pt(c - self.origin), radiusX: r, radiusY: r };
            self.dc.FillEllipse(&e, &self.solid);
        }
    }

    fn line(&self, a: V2, b: V2, width: f32, col: D2D1_COLOR_F) {
        unsafe {
            self.solid.SetColor(&col);
            self.dc.DrawLine(pt(a - self.origin), pt(b - self.origin), &self.solid, width, None);
        }
    }

    fn text(&self, s: &str, center: V2, fmt: &IDWriteTextFormat, col: D2D1_COLOR_F, shadow: f32) {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let c = center - self.origin;
        let rect = |dx: f32, dy: f32| D2D_RECT_F {
            left: c.x - 2000.0 + dx,
            top: c.y - 300.0 + dy,
            right: c.x + 2000.0 + dx,
            bottom: c.y + 300.0 + dy,
        };
        unsafe {
            if shadow > 0.0 {
                self.solid.SetColor(&color(0.02, 0.05, 0.12, col.a * 0.75));
                for (dx, dy) in [(shadow, shadow), (-shadow * 0.5, shadow), (shadow, -shadow * 0.5)] {
                    self.dc.DrawText(
                        &wide,
                        fmt,
                        &rect(dx, dy),
                        &self.solid,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                }
            }
            self.solid.SetColor(&col);
            self.dc.DrawText(
                &wide,
                fmt,
                &rect(0.0, 0.0),
                &self.solid,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn speckles(&self, c: V2, r: f32, rot: f32, opacity: f32) {
        if opacity <= 0.01 || r < 6.0 {
            return;
        }
        let c = c - self.origin;
        unsafe {
            self.speck.SetOpacity(opacity);
            for &(a, d, sz) in SPECKLES.iter() {
                let p = c + V2::from_angle(a + rot) * (d * r * 0.85);
                let fade = 1.0 - d * d * 0.6;
                let e = D2D1_ELLIPSE { point: pt(p), radiusX: sz * r * fade, radiusY: sz * r * fade };
                self.dc.FillEllipse(&e, &self.speck);
            }
        }
    }

    // ---------------------------------------------------------------- scene

    fn draw_world(&self, w: &World) -> Result<()> {
        // menus and flyouts may sit below our overlay: keep their area free of snow
        let masked = !w.popups.is_empty() || !w.shields.is_empty();
        if masked {
            self.push_popup_mask(w)?;
        }
        let r = self.draw_scene(w);
        if masked {
            unsafe { self.dc.PopLayer() };
        }
        r
    }

    fn push_popup_mask(&self, w: &World) -> Result<()> {
        let o = self.origin;
        let vr = w.vrect;
        let rect = |l: f32, t: f32, r: f32, b: f32| D2D_RECT_F {
            left: l - o.x,
            top: t - o.y,
            right: r - o.x,
            bottom: b - o.y,
        };
        unsafe {
            let mut geoms: Vec<Option<ID2D1Geometry>> = Vec::with_capacity(w.popups.len() + 1);
            let full = rect(vr.l as f32, vr.t as f32, vr.r as f32, vr.b as f32);
            geoms.push(Some(self.factory.CreateRectangleGeometry(&full)?.cast()?));
            for p in w.popups.iter().chain(&w.shields) {
                // a little margin for rounded corners and the edge of the shadow
                let g = self.factory.CreateRectangleGeometry(&rect(
                    p.l as f32 - 2.0,
                    p.t as f32 - 2.0,
                    p.r as f32 + 2.0,
                    p.b as f32 + 2.0,
                ))?;
                geoms.push(Some(g.cast()?));
            }
            // alternate fill: the screen with holes where the popups are
            let group = self.factory.CreateGeometryGroup(D2D1_FILL_MODE_ALTERNATE, &geoms)?;
            let params = D2D1_LAYER_PARAMETERS1 {
                contentBounds: D2D_RECT_F { left: -1e7, top: -1e7, right: 1e7, bottom: 1e7 },
                geometricMask: std::mem::ManuallyDrop::new(Some(group.cast()?)),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_ALIASED,
                maskTransform: windows_numerics::Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: std::mem::ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            self.dc.PushLayer(&params, None);
            // the layer holds its own reference to the mask
            drop(std::mem::ManuallyDrop::into_inner(params.geometricMask));
        }
        Ok(())
    }

    fn draw_scene(&self, w: &World) -> Result<()> {
        let s = w.scale;
        // splats stuck on windows
        for m in &w.marks {
            let a = (m.life / 3.0).min(1.0) * 0.95;
            let off = w.window_vel(m.hwnd) * w.lead;
            let o = self.origin - off;
            // on its own window only, and never over windows in front of it
            let area = Area {
                l: m.clip.l as f32 - o.x,
                t: m.clip.t as f32 - o.y,
                r: m.clip.r as f32 - o.x,
                b: m.clip.b as f32 - o.y,
            };
            let front = self.areas(&w.front_of(m.hwnd));
            self.occluded(area, &front, || {
                for &(bo, sz) in &m.blobs {
                    self.sprite(&self.ball, m.pos + off + bo * m.r, sz * m.r, a);
                }
                Ok(())
            })?;
        }

        let td = Instant::now();
        self.draw_drifts(w)?;
        let t = self.timing.get();
        self.timing.set([t[0], t[1], td.elapsed().as_secs_f32() * 1000.0]);

        // stacks whose base is covered are still drawn: windows in front clip them
        for st in &w.stacks {
            let off = st.vel * w.lead;
            let front = st.window().map(|h| self.areas(&w.front_of(h))).unwrap_or_default();
            // bounds of the whole snowman: arms reach two radii out, the hat two radii up
            let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (ball, c) in st.balls.iter().zip(&st.centers) {
                let c = *c + off - self.origin;
                l = l.min(c.x - ball.r * 2.2);
                r = r.max(c.x + ball.r * 2.2);
                t = t.min(c.y - ball.r * 2.0);
                b = b.max(c.y + ball.r * 1.9);
            }
            if l < r {
                self.occluded(Area { l, t, r, b }, &front, || self.draw_stack(st, off))?;
            }
        }

        // a carried snowman is in the hand, in front of every window
        if let Some(c) = &w.carried {
            self.draw_stack(&c.stack, c.stack.vel * w.lead)?;
        }

        if let Some(b) = &w.firebox {
            self.draw_firebox(w, b)?;
        }

        for p in &w.plows {
            self.draw_plow(w, p)?;
        }

        let mut flakes = Sprites::default();
        let mut chunks = Sprites::default();
        for p in &w.particles {
            match p.kind {
                Kind::Flake => flakes.push(p.pos, p.r * 1.9, p.alpha),
                Kind::Chunk => chunks.push(p.pos, p.r, 1.0),
                Kind::Spray => flakes.push(p.pos, p.r * 2.0, p.alpha.min(1.0)),
            }
        }
        self.flush(&mut flakes, &self.flake);
        self.flush(&mut chunks, &self.ball);

        for b in &w.balls {
            self.sprite(&self.ball, b.pos, b.r, 1.0);
            self.speckles(b.pos, b.r, b.rot, 0.35);
        }

        // the snow in hand: loose lumps that merge into a ball as it gets rolled
        if let Some(h) = &w.held {
            let r = ball_radius(h.mass, s);
            let c = h.compaction;
            let spread = (1.0 - c).powf(1.5);
            if spread > 0.01 && !h.blobs.is_empty() {
                for &(off, sz) in &h.blobs {
                    let rot = V2::from_angle(h.rot * 0.15);
                    let o = V2(off.x * rot.x - off.y * rot.y, off.x * rot.y + off.y * rot.x);
                    self.sprite(&self.ball, h.pos + o * (r * 0.9 * spread), r * lerp(sz, 1.0, c), 1.0);
                }
            } else {
                self.sprite(&self.ball, h.pos, r, 1.0);
            }
            self.speckles(h.pos, r, h.rot, ((c - 0.4) * 1.5).clamp(0.0, 0.35));
        }

        self.draw_fireworks(w);

        if let Some(b) = &w.banner {
            let a = b.alpha.clamp(0.0, 1.0);
            self.text(&b.text, b.center, &self.banner_fmt, color(1.0, 0.97, 0.85, a), 4.0 * s);
            if let Some(sub) = &b.sub {
                self.text(sub, b.center + V2(0.0, 110.0 * s), &self.sub_fmt, color(1.0, 0.85, 0.35, a), 3.0 * s);
            }
        }
        Ok(())
    }

    /// Redraws changed segments of each drift's texture. Runs before the main pass.
    fn update_drift_textures(&self, w: &World) -> Result<()> {
        let now = Instant::now();
        let mut cache = self.drift_cache.borrow_mut();
        for c in cache.values_mut() {
            c.seen = false;
        }
        let mut drew = false;
        for surf in w.surfaces() {
            let width = surf.heights.len();
            let height = (surf.max_h + 4.0 * w.scale).ceil() as u32;
            let m = surf.seg_rev.len();
            let stale = cache.get(&surf.key).is_none_or(|t| t.width != width || t.height != height);
            if stale {
                let props = D2D1_BITMAP_PROPERTIES1 { bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET, ..sprite_props() };
                let bmp = unsafe { self.dc.CreateBitmap(D2D_SIZE_U { width: width as u32, height }, None, 0, &props)? };
                cache.insert(
                    surf.key,
                    DriftTex { bmp, width, height, seg_rev: vec![0; m], built: vec![now; m], seen: true },
                );
            }
            let t = cache.get_mut(&surf.key).unwrap();
            t.seen = true;
            t.seg_rev.resize(m, 0);
            t.built.resize(m, now);
            let todo: Vec<usize> = (0..m)
                .filter(|&k| {
                    let rev = surf.seg_rev[k];
                    t.seg_rev[k] == 0
                        || (t.seg_rev[k] != rev
                            && (surf.seg_urgent[k] > t.seg_rev[k] || now - t.built[k] >= DRIFT_REBUILD))
                })
                .collect();
            if todo.is_empty() {
                continue;
            }
            unsafe {
                self.dc.SetTarget(&t.bmp);
                self.dc.BeginDraw();
                for &k in &todo {
                    DRIFT_BUILDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let (s0, s1) = (k * SEG, ((k + 1) * SEG).min(width));
                    let clip = D2D_RECT_F { left: s0 as f32, top: 0.0, right: s1 as f32, bottom: height as f32 };
                    self.dc.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
                    self.dc.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
                    // draw a few columns past the clip on both sides so outlines join seamlessly
                    self.draw_drift_span(surf, s0.saturating_sub(4), (s1 + 4).min(width), height as f32, w.scale)?;
                    self.dc.PopAxisAlignedClip();
                    t.seg_rev[k] = surf.seg_rev[k];
                    t.built[k] = now;
                }
                self.dc.EndDraw(None, None)?;
            }
            drew = true;
        }
        if drew {
            unsafe { self.dc.SetTarget(&self.target_bmp) };
        }
        cache.retain(|_, c| c.seen);
        Ok(())
    }

    /// Fills and outlines the drift over columns a..b, in texture coordinates (base at y = `base`).
    fn draw_drift_span(&self, surf: &Surface, a0: usize, b0: usize, base: f32, scale: f32) -> Result<()> {
        // the whole drift is drawn, even where a window in front covers its base line:
        // windows in front clip it when it is drawn, so it is hidden exactly up to their edge
        let hidden = surf.hidden;
        let snowy = |i: usize| !hidden && surf.heights[i] >= 0.3;
        let mut pts: Vec<Vector2> = Vec::with_capacity(SEG / 2 + 8);
        unsafe {
            let fill = self.factory.CreatePathGeometry()?;
            let line = self.factory.CreatePathGeometry()?;
            let fs = fill.Open()?;
            let ls = line.Open()?;
            let mut any = false;
            let mut i = a0;
            while i < b0 {
                if !snowy(i) {
                    i += 1;
                    continue;
                }
                let a = i;
                while i < b0 && snowy(i) {
                    i += 1;
                }
                let b = i;
                any = true;
                pts.clear();
                let mut c = a;
                while c < b {
                    pts.push(Vector2 { X: c as f32 + 0.5, Y: base - surf.heights[c] });
                    c += 2;
                }
                pts.push(Vector2 { X: b as f32 - 0.5, Y: base - surf.heights[b - 1] });
                fs.BeginFigure(Vector2 { X: a as f32, Y: base }, D2D1_FIGURE_BEGIN_FILLED);
                fs.AddLines(&pts);
                fs.AddLine(Vector2 { X: b as f32, Y: base });
                fs.EndFigure(D2D1_FIGURE_END_CLOSED);
                // vertical edges only where the drift really ends, not where it joins a neighbouring drift
                let n = surf.heights.len();
                let edge_l = a > 0 || !surf.join_l;
                let edge_r = b < n || !surf.join_r;
                let first = if edge_l { Vector2 { X: a as f32, Y: base } } else { pts[0] };
                ls.BeginFigure(first, D2D1_FIGURE_BEGIN_HOLLOW);
                ls.AddLines(if edge_l { &pts[..] } else { &pts[1..] });
                if edge_r {
                    ls.AddLine(Vector2 { X: b as f32, Y: base });
                }
                ls.EndFigure(D2D1_FIGURE_END_OPEN);
            }
            fs.Close()?;
            ls.Close()?;
            if any {
                self.dc.FillGeometry(&fill, &self.pile, None);
                self.dc.DrawGeometry(&line, &self.edge, scale.max(1.0), None);
            }
        }
        Ok(())
    }

    /// Back to front, so a drift in front covers the one behind it. One textured quad per drift.
    fn draw_drifts(&self, w: &World) -> Result<()> {
        let cache = self.drift_cache.borrow();
        for surf in w.surfaces() {
            let Some(t) = cache.get(&surf.key) else { continue };
            // a moving window's drift goes where the window will be when this frame shows up;
            // whole pixels keep the texture crisp
            let off = (surf.vel * w.lead) - self.origin;
            let x = (surf.left as f32 + off.x).round();
            let y = (surf.top as f32 - t.height as f32 + off.y).round();
            let dest = D2D_RECT_F { left: x, top: y, right: x + t.width as f32, bottom: y + t.height as f32 };
            // the drift rises above its window's edge: windows in front hide it, not only its base
            let front = if surf.key < (1u64 << 62) { self.areas(&w.front_of(surf.key as isize)) } else { Vec::new() };
            let area = Area { l: dest.left, t: dest.top, r: dest.right, b: dest.bottom };
            self.occluded(area, &front, || {
                unsafe {
                    self.dc.DrawBitmap(&t.bmp, Some(&dest), 1.0, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None, None);
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn draw_stack(&self, st: &Stack, off: V2) -> Result<()> {
        let snowman = st.is_snowman() && st.centers.len() >= 3;
        // twig arms go behind the body, so draw them first
        if snowman {
            let body = st.centers[1] + off;
            let rb = st.balls[1].r;
            let a = rb * 1.45;
            let ah = a * self.arm_img.h / self.arm_img.w;
            self.image(&self.arm_img, body.x + rb * 0.62, body.y + rb * 0.2 - ah, a, ah, false);
            self.image(&self.arm_img, body.x - rb * 0.62 - a, body.y + rb * 0.2 - ah, a, ah, true);
        }
        for (b, c) in st.balls.iter().zip(&st.centers) {
            self.sprite(&self.ball, *c + off, b.r, 1.0);
            self.speckles(*c + off, b.r, b.rot, 0.25);
        }
        if !snowman {
            return Ok(());
        }
        let body = st.centers[1] + off;
        let rb = st.balls[1].r;
        let head = st.centers[2] + off;
        let rh = st.balls[2].r;
        // coal buttons
        let coal = color(0.12, 0.12, 0.15, 1.0);
        for k in [-0.42f32, 0.02, 0.46] {
            let c = body + V2(0.0, rb * k);
            self.fill_circle(c, (rb * 0.085).max(1.5), coal);
            self.fill_circle(c + V2(-rb * 0.025, -rb * 0.025), (rb * 0.03).max(0.6), color(0.45, 0.45, 0.5, 1.0));
        }
        // knitted scarf around the neck, end hanging on the right
        let sw = rh * 2.3;
        let look = st.look;
        let scarf = &self.scarves[(look / 4) as usize % self.scarves.len()];
        let sh = sw * scarf.h / scarf.w;
        self.image(scarf, head.x - sw * 0.5, head.y + rh * 0.62, sw, sh, false);
        // coal face and carrot nose
        let fw = rh * 1.1;
        let face = &self.faces[(look / 12) as usize % self.faces.len()];
        let fh = fw * face.h / face.w;
        self.image(face, head.x - fw * 0.5, head.y - rh * 0.45, fw, fh, false);
        let nw = rh * 0.8;
        let nh = nw * self.nose_img.h / self.nose_img.w;
        self.image(&self.nose_img, head.x - rh * 0.05, head.y - nh * 0.35, nw, nh, false);
        // the hat sits on top of the head
        // (sprite, width in head radii, how far down onto the head it sits)
        let fits = [(2.05, 0.55), (2.0, 0.35), (1.9, 0.38), (1.95, 0.5)];
        let k = look as usize % self.hats.len();
        let (wk, sink) = fits[k];
        let hat = &self.hats[k];
        let hw = rh * wk;
        let hh = hw * hat.h / hat.w;
        self.image(hat, head.x - hw * 0.5, head.y - rh * sink - hh, hw, hh, false);
        Ok(())
    }

    fn draw_firebox(&self, w: &World, b: &FireBox) -> Result<()> {
        let s = w.scale;
        let (x, y, bw, bh) = (b.pos.x, b.pos.y, b.w, b.h);
        let iw = bw * 1.08;
        let ih = iw * self.box_img.h / self.box_img.w;
        self.image(&self.box_img, x - iw / 2.0, y - ih, iw, ih, false);
        // a lit fuse while a show is running
        if w.fireworks.active() {
            let p = V2(x + bw * 0.28, y - bh - 2.0 * s);
            self.sprite(&self.glow[1], p, (6.0 + 3.0 * (w.time * 25.0).sin()) * s, 1.0);
        }
        if let Some(label) = &b.label {
            self.text(label, V2(x, y - ih - 14.0 * s), &self.label_fmt, color(1.0, 0.95, 0.8, 0.95), 1.2 * s);
        }
        Ok(())
    }

    fn draw_plow(&self, w: &World, p: &Plow) -> Result<()> {
        let s = w.scale;
        let Some(f) = w.floors.get(p.floor) else { return Ok(()) };
        let base = f.top as f32;
        let d = p.dir;
        // the sprite faces right; mirrored when driving left. A little bounce on the bumps.
        let pw = 2.0 * (fx::PLOW_BLADE + 2.0) * s;
        let ph = pw * self.plow_img.h / self.plow_img.w;
        let bounce = (p.t * 13.0).sin().abs() * 1.2 * s;
        let (l, t) = (p.x - pw / 2.0, base - ph - bounce + 2.0 * s);
        // snow pushed ahead of the blade
        let load = (p.load.sqrt() * 0.7 * s).min(16.0 * s);
        if load > 1.0 {
            self.sprite(&self.ball, V2(p.x + d * (pw / 2.0 + load * 0.4), base - load * 0.6), load, 1.0);
        }
        self.image(&self.plow_img, l, t, pw, ph, d < 0.0);
        // amber beacon on the cab roof
        let blink = (p.t * 7.0).sin() > 0.0;
        if blink {
            let bx = p.x + d * (0.53 - 0.5) * pw;
            self.sprite(&self.glow[1], V2(bx, t + ph * 0.06), 20.0 * s, 0.9);
        }
        Ok(())
    }

    fn draw_fireworks(&self, w: &World) {
        let fw = &w.fireworks;
        if fw.sparks.is_empty() && fw.rockets.is_empty() {
            return;
        }
        let s = w.scale;
        unsafe {
            self.dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_ADD);
        }
        let mut by_color: Vec<Sprites> = (0..self.glow.len()).map(|_| Sprites::default()).collect();
        for (i, sp) in fw.sparks.iter().enumerate() {
            let mut a = (sp.life / sp.max_life).clamp(0.0, 1.0).powf(0.7);
            if sp.twinkle && (w.time * 30.0 + i as f32 * 1.7).sin() < 0.0 {
                a *= 0.15;
            }
            by_color[sp.color as usize % self.glow.len()].push(sp.pos, sp.size * 2.2, a);
        }
        for (k, mut list) in by_color.into_iter().enumerate() {
            self.flush(&mut list, &self.glow[k]);
        }
        for r in &fw.rockets {
            self.sprite(&self.glow[7], r.pos, 4.5 * s, 1.0);
            self.sprite(&self.glow[r.color as usize % self.glow.len()], r.pos, 9.0 * s, 0.5);
        }
        unsafe {
            self.dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
        }
    }
}
