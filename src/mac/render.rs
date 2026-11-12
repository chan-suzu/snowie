//! macOS renderer: the same picture as the Direct2D one on Windows, drawn with wgpu (Metal)
//! into one transparent layer per screen. Everything is textured triangles: a frame is built
//! once as a vertex list in global screen points, then drawn into every screen's surface with
//! that screen's origin, clipped by scissor rectangles where windows in front hide the snow.

use crate::fx::{self, Plow, PALETTE};
use crate::geom::{lerp, RectI, V2};
use crate::world::{ball_radius, FireBox, Kind, Stack, World, SPECKLES};
use raw_window_handle::{AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::collections::HashMap;
use std::ptr::NonNull;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    col: [f32; 4],
}

#[derive(Clone, Copy, PartialEq)]
enum Blend {
    Over,
    Add,
}

/// An axis-aligned area in global screen points.
#[derive(Clone, Copy, PartialEq)]
struct Area {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
}

fn areas(rects: &[RectI]) -> Vec<Area> {
    rects.iter().map(|r| Area { l: r.l as f32, t: r.t as f32, r: r.r as f32, b: r.b as f32 }).collect()
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

/// A run of vertices drawn with one texture, blend mode and clip.
struct Cmd {
    tex: usize,
    blend: Blend,
    clip: Option<Area>,
    start: u32,
    end: u32,
}

struct Tex {
    bind: wgpu::BindGroup,
    texture: wgpu::Texture,
    w: f32,
    h: f32,
}

// ---------------------------------------------------------------- generated sprites

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Premultiplied RGBA.
fn put(v: &mut [u8], i: usize, r: f32, g: f32, b: f32, a: f32) {
    let a = a.clamp(0.0, 1.0);
    v[i] = (r.clamp(0.0, 1.0) * a * 255.0) as u8;
    v[i + 1] = (g.clamp(0.0, 1.0) * a * 255.0) as u8;
    v[i + 2] = (b.clamp(0.0, 1.0) * a * 255.0) as u8;
    v[i + 3] = (a * 255.0) as u8;
}

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

/// A white antialiased disc, tinted per draw (coal buttons, speckles).
fn disc_pixels(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) / 2.0;
    let rad = n as f32 / 2.0 - 1.0;
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            put(&mut v, (y * n + x) * 4, 1.0, 1.0, 1.0, (rad - d + 0.5).clamp(0.0, 1.0));
        }
    }
    v
}

/// Decodes an embedded PNG to premultiplied RGBA.
fn decode_png(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
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
        px[i * 4] = m(r);
        px[i * 4 + 1] = m(g);
        px[i * 4 + 2] = m(b);
        px[i * 4 + 3] = a;
    }
    Some((w, h, px))
}

// ---------------------------------------------------------------- text

const ATLAS: u32 = 2048;

#[derive(Clone, Copy)]
struct Glyph {
    /// atlas rectangle in pixels
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    xmin: f32,
    ymin: f32,
    advance: f32,
}

/// Glyphs rasterized on demand into one atlas texture (white, alpha = coverage).
struct Glyphs {
    font: fontdue::Font,
    map: HashMap<(char, u32), Glyph>,
    pixels: Vec<u8>,
    pen: (u32, u32, u32),
    dirty: bool,
}

impl Glyphs {
    fn load() -> Option<Glyphs> {
        let candidates = [
            "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/Library/Fonts/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ];
        let bytes = candidates.iter().find_map(|p| std::fs::read(p).ok())?;
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()?;
        Some(Glyphs {
            font,
            map: HashMap::new(),
            pixels: vec![0u8; (ATLAS * ATLAS * 4) as usize],
            pen: (1, 1, 0),
            dirty: false,
        })
    }

    fn get(&mut self, ch: char, px: u32) -> Option<Glyph> {
        if let Some(g) = self.map.get(&(ch, px)) {
            return Some(*g);
        }
        let (m, cov) = self.font.rasterize(ch, px as f32);
        let (w, h) = (m.width as u32, m.height as u32);
        if w + 2 > ATLAS || h + 2 > ATLAS {
            return None;
        }
        let (mut x, mut y, mut row) = self.pen;
        if x + w + 1 > ATLAS {
            x = 1;
            y += row + 1;
            row = 0;
        }
        if y + h + 1 > ATLAS {
            // full: start over (rare: only very many different sizes would fill it)
            self.map.clear();
            self.pixels.fill(0);
            x = 1;
            y = 1;
            row = 0;
        }
        for gy in 0..h {
            for gx in 0..w {
                let a = cov[(gy * w + gx) as usize];
                let i = (((y + gy) * ATLAS + x + gx) * 4) as usize;
                self.pixels[i..i + 4].copy_from_slice(&[a, a, a, a]);
            }
        }
        let g = Glyph { x, y, w, h, xmin: m.xmin as f32, ymin: m.ymin as f32, advance: m.advance_width };
        self.pen = (x + w + 1, y, row.max(h));
        self.map.insert((ch, px), g);
        self.dirty = true;
        Some(g)
    }
}

// ---------------------------------------------------------------- renderer

/// One screen: its own transparent surface.
pub struct Target {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    msaa: wgpu::TextureView,
    /// the screen in global points, and its pixels per point
    origin: V2,
    size: V2,
    scale: f32,
    ubuf: wgpu::Buffer,
    ubind: wgpu::BindGroup,
}

const MSAA: u32 = 4;

struct Ids {
    white: usize,
    disc: usize,
    flake: usize,
    ball: usize,
    glow: Vec<usize>,
    plow: usize,
    firebox: usize,
    arm: usize,
    nose: usize,
    hats: Vec<usize>,
    scarves: Vec<usize>,
    faces: Vec<usize>,
    atlas: usize,
}

pub struct Renderer {
    /// kept alive for the surfaces
    _instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    alpha_mode: wgpu::CompositeAlphaMode,
    pipe_over: wgpu::RenderPipeline,
    pipe_add: wgpu::RenderPipeline,
    tex_layout: wgpu::BindGroupLayout,
    uni_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: Vec<Tex>,
    ids: Ids,
    glyphs: Option<Glyphs>,
    verts: Vec<Vertex>,
    cmds: Vec<Cmd>,
    vbuf: wgpu::Buffer,
    blend: Blend,
    clip: Option<Area>,
    pub targets: Vec<Target>,
    /// largest pixels-per-point of all screens: text and lines are sized for it
    max_scale: f32,
}

const SHADER: &str = r#"
struct U { origin: vec2<f32>, size: vec2<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VO {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) col: vec4<f32>,
};

@vertex
fn vs(@location(0) p: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>) -> VO {
    var o: VO;
    let n = (p - u.origin) / u.size;
    o.pos = vec4<f32>(n.x * 2.0 - 1.0, 1.0 - n.y * 2.0, 0.0, 1.0);
    o.uv = uv;
    o.col = col;
    return o;
}

@fragment
fn fs(i: VO) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, i.uv) * i.col;
}
"#;

/// Premultiplied colour.
fn rgba(r: f32, g: f32, b: f32, a: f32) -> [f32; 4] {
    [r * a, g * a, b * a, a]
}

impl Renderer {
    /// `views` are the content views of the overlay panels: (NSView pointer, origin, size in
    /// points, pixels per point).
    pub fn new(views: &[(NonNull<std::ffi::c_void>, V2, V2, f32)]) -> Result<Renderer, String> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::METAL;
        let instance = wgpu::Instance::new(desc);
        let mut surfaces = Vec::new();
        for (view, ..) in views {
            let handle = RawWindowHandle::AppKit(AppKitWindowHandle::new(*view));
            let surface = unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(RawDisplayHandle::AppKit(AppKitDisplayHandle::new())),
                    raw_window_handle: handle,
                })
            }
            .map_err(|e| format!("surface: {e}"))?;
            surfaces.push(surface);
        }
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: surfaces.first(),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("adapter: {e}"))?;
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor { label: Some("snowie"), ..Default::default() }),
        )
        .map_err(|e| format!("device: {e}"))?;
        let caps = surfaces.first().ok_or("no screens")?.get_capabilities(&adapter);
        // a plain (non-sRGB) format: colours match the Direct2D version
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or("no surface formats")?;
        let alpha_mode = [wgpu::CompositeAlphaMode::PostMultiplied, wgpu::CompositeAlphaMode::PreMultiplied]
            .into_iter()
            .find(|m| caps.alpha_modes.contains(m))
            .unwrap_or(caps.alpha_modes[0]);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("snowie"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uni_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&uni_layout), Some(&tex_layout)],
            immediate_size: 0,
        });
        let vertex_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
        let pipeline = |blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &vertex_attrs,
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let over = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        // fireworks light up whatever is behind them; the overlay's alpha stays as it is
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipe_over = pipeline(over);
        let pipe_add = pipeline(add);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 1 << 20,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let max_scale = views.iter().map(|v| v.3).fold(1.0, f32::max);
        let mut r = Renderer {
            _instance: instance,
            device,
            queue,
            format,
            alpha_mode,
            pipe_over,
            pipe_add,
            tex_layout,
            uni_layout,
            sampler,
            textures: Vec::new(),
            ids: Ids {
                white: 0,
                disc: 0,
                flake: 0,
                ball: 0,
                glow: Vec::new(),
                plow: 0,
                firebox: 0,
                arm: 0,
                nose: 0,
                hats: Vec::new(),
                scarves: Vec::new(),
                faces: Vec::new(),
                atlas: 0,
            },
            glyphs: Glyphs::load(),
            verts: Vec::with_capacity(1 << 16),
            cmds: Vec::new(),
            vbuf,
            blend: Blend::Over,
            clip: None,
            targets: Vec::new(),
            max_scale,
        };
        r.ids.white = r.texture(1, 1, &[255, 255, 255, 255]);
        r.ids.disc = r.texture(64, 64, &disc_pixels(64));
        r.ids.flake = r.texture(32, 32, &flake_pixels(32));
        r.ids.ball = r.texture(256, 256, &ball_pixels(256));
        r.ids.glow = PALETTE.iter().map(|c| r.texture(32, 32, &glow_pixels(32, *c))).collect();
        let mut png = |bytes: &[u8]| {
            let (w, h, px) = decode_png(bytes).unwrap_or((1, 1, vec![0; 4]));
            r.texture(w as u32, h as u32, &px)
        };
        let plow = png(include_bytes!("../../assets/sprites/plow.png"));
        let firebox = png(include_bytes!("../../assets/sprites/box.png"));
        let arm = png(include_bytes!("../../assets/sprites/arm.png"));
        let nose = png(include_bytes!("../../assets/sprites/nose.png"));
        let hats = vec![
            png(include_bytes!("../../assets/sprites/hat.png")),
            png(include_bytes!("../../assets/sprites/hat_santa.png")),
            png(include_bytes!("../../assets/sprites/hat_beanie.png")),
            png(include_bytes!("../../assets/sprites/hat_bucket.png")),
        ];
        let scarves = vec![
            png(include_bytes!("../../assets/sprites/scarf.png")),
            png(include_bytes!("../../assets/sprites/scarf_blue.png")),
            png(include_bytes!("../../assets/sprites/scarf_green.png")),
        ];
        let faces = vec![
            png(include_bytes!("../../assets/sprites/face.png")),
            png(include_bytes!("../../assets/sprites/face_wink.png")),
            png(include_bytes!("../../assets/sprites/face_grin.png")),
        ];
        r.ids.plow = plow;
        r.ids.firebox = firebox;
        r.ids.arm = arm;
        r.ids.nose = nose;
        r.ids.hats = hats;
        r.ids.scarves = scarves;
        r.ids.faces = faces;
        r.ids.atlas = r.texture(ATLAS, ATLAS, &vec![0u8; (ATLAS * ATLAS * 4) as usize]);

        for (surface, &(_, origin, size, scale)) in surfaces.into_iter().zip(views) {
            let t = r.target(surface, origin, size, scale);
            r.targets.push(t);
        }
        Ok(r)
    }

    fn texture(&mut self, w: u32, h: u32, px: &[u8]) -> usize {
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.upload(&texture, w, h, px);
        let view = texture.create_view(&Default::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.textures.push(Tex { bind, texture, w: w as f32, h: h as f32 });
        self.textures.len() - 1
    }

    fn upload(&self, texture: &wgpu::Texture, w: u32, h: u32, px: &[u8]) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            px,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    fn target(&self, surface: wgpu::Surface<'static>, origin: V2, size: V2, scale: f32) -> Target {
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.format,
            width: ((size.x * scale) as u32).max(1),
            height: ((size.y * scale) as u32).max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 1,
            alpha_mode: self.alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&self.device, &config);
        let msaa = self.msaa_view(&config);
        let ubuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ubind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.uni_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
        });
        Target { surface, config, msaa, origin, size, scale, ubuf, ubind }
    }

    fn msaa_view(&self, config: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
        self.device
            .create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: MSAA,
                dimension: wgpu::TextureDimension::D2,
                format: config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    // ---------------------------------------------------------------- recording

    fn push_cmd(&mut self, tex: usize) {
        let n = self.verts.len() as u32;
        if let Some(c) = self.cmds.last_mut() {
            if c.tex == tex && c.blend == self.blend && c.clip == self.clip && c.end == n {
                return;
            }
        }
        self.cmds.push(Cmd { tex, blend: self.blend, clip: self.clip, start: n, end: n });
    }

    fn close_cmd(&mut self) {
        let n = self.verts.len() as u32;
        if let Some(c) = self.cmds.last_mut() {
            c.end = n;
        }
    }

    /// A textured quad given by its four corners (clockwise from top-left).
    fn quad4(&mut self, tex: usize, p: [V2; 4], uv: [f32; 4], col: [f32; 4]) {
        self.push_cmd(tex);
        let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
        let v = |q: V2, u: f32, w: f32| Vertex { pos: [q.x, q.y], uv: [u, w], col };
        let (a, b, c, d) = (v(p[0], u0, v0), v(p[1], u1, v0), v(p[2], u1, v1), v(p[3], u0, v1));
        self.verts.extend_from_slice(&[a, b, c, a, c, d]);
        self.close_cmd();
    }

    fn rect(&mut self, tex: usize, a: Area, uv: [f32; 4], col: [f32; 4]) {
        let Area { l, t, r, b } = a;
        self.quad4(tex, [V2(l, t), V2(r, t), V2(r, b), V2(l, b)], uv, col);
    }

    fn sprite(&mut self, tex: usize, c: V2, r: f32, alpha: f32) {
        self.rect(tex, Area { l: c.x - r, t: c.y - r, r: c.x + r, b: c.y + r }, [0.0, 0.0, 1.0, 1.0], [alpha; 4]);
    }

    fn image(&mut self, tex: usize, l: f32, t: f32, w: f32, h: f32, flip: bool) {
        let uv = if flip { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] };
        self.rect(tex, Area { l, t, r: l + w, b: t + h }, uv, [1.0; 4]);
    }

    fn circle(&mut self, c: V2, r: f32, col: [f32; 4]) {
        let disc = self.ids.disc;
        self.rect(disc, Area { l: c.x - r, t: c.y - r, r: c.x + r, b: c.y + r }, [0.0, 0.0, 1.0, 1.0], col);
    }

    fn tri(&mut self, a: V2, b: V2, c: V2, col: [f32; 4]) {
        let white = self.ids.white;
        self.push_cmd(white);
        let v = |q: V2| Vertex { pos: [q.x, q.y], uv: [0.5, 0.5], col };
        self.verts.extend_from_slice(&[v(a), v(b), v(c)]);
        self.close_cmd();
    }

    fn line(&mut self, a: V2, b: V2, width: f32, col: [f32; 4]) {
        let d = b - a;
        let len = d.len();
        if len < 1e-3 {
            return;
        }
        let n = V2(-d.y, d.x) * (width * 0.5 / len);
        let white = self.ids.white;
        self.quad4(white, [a + n, b + n, b - n, a - n], [0.5, 0.5, 0.5, 0.5], col);
    }

    fn aspect(&self, tex: usize) -> f32 {
        let t = &self.textures[tex];
        t.h / t.w.max(1.0)
    }

    /// Text centred on `center`; `size` in points.
    fn text(&mut self, s: &str, center: V2, size: f32, col: [f32; 4], shadow: f32) {
        let px = (size * self.max_scale).round().max(4.0) as u32;
        let k = 1.0 / self.max_scale;
        let Some(glyphs) = self.glyphs.as_mut() else { return };
        let laid: Vec<Glyph> = s.chars().filter_map(|c| glyphs.get(c, px)).collect();
        let width: f32 = laid.iter().map(|g| g.advance).sum::<f32>() * k;
        let line = glyphs.font.horizontal_line_metrics(px as f32);
        let (ascent, descent) = line.map_or((px as f32 * 0.8, -(px as f32) * 0.2), |m| (m.ascent, m.descent));
        let atlas = self.ids.atlas;
        let a = ATLAS as f32;
        let draw = |r: &mut Renderer, dx: f32, dy: f32, col: [f32; 4]| {
            let mut x = center.x - width / 2.0 + dx;
            let base = center.y + (ascent + descent) * k / 2.0 + dy;
            for g in &laid {
                if g.w > 0 && g.h > 0 {
                    let l = x + g.xmin * k;
                    let b = base - g.ymin * k;
                    let t = b - g.h as f32 * k;
                    let uv = [g.x as f32 / a, g.y as f32 / a, (g.x + g.w) as f32 / a, (g.y + g.h) as f32 / a];
                    r.rect(atlas, Area { l, t, r: l + g.w as f32 * k, b }, uv, col);
                }
                x += g.advance * k;
            }
        };
        if shadow > 0.0 {
            let sc = rgba(0.02, 0.05, 0.12, col[3] * 0.75);
            for (dx, dy) in [(shadow, shadow), (-shadow * 0.5, shadow), (shadow, -shadow * 0.5)] {
                draw(self, dx, dy, sc);
            }
        }
        draw(self, 0.0, 0.0, col);
    }

    fn speckles(&mut self, c: V2, r: f32, rot: f32, opacity: f32) {
        if opacity <= 0.01 || r < 6.0 {
            return;
        }
        let col = rgba(0.55, 0.62, 0.74, 0.3 * opacity);
        for &(a, d, sz) in SPECKLES.iter() {
            let p = c + V2::from_angle(a + rot) * (d * r * 0.85);
            let fade = 1.0 - d * d * 0.6;
            self.circle(p, sz * r * fade, col);
        }
    }

    /// Runs `draw` clipped to `area` minus the windows in `front`.
    fn occluded(&mut self, area: Area, front: &[Area], draw: impl Fn(&mut Renderer)) {
        let saved = self.clip;
        if front.is_empty() {
            self.clip = Some(area);
            draw(self);
        } else {
            for p in subtract(area, front) {
                self.clip = Some(p);
                draw(self);
            }
        }
        self.clip = saved;
    }

    // ---------------------------------------------------------------- scene

    fn scene(&mut self, w: &World) {
        let s = w.scale;
        // splats stuck on windows
        for m in &w.marks {
            let a = (m.life / 3.0).min(1.0) * 0.95;
            let area = Area { l: m.clip.l as f32, t: m.clip.t as f32, r: m.clip.r as f32, b: m.clip.b as f32 };
            let front = areas(&w.front_of(m.hwnd));
            let ball = self.ids.ball;
            self.occluded(area, &front, |r| {
                for &(bo, sz) in &m.blobs {
                    r.sprite(ball, m.pos + bo * m.r, sz * m.r, a);
                }
            });
        }

        self.drifts(w);

        for st in &w.stacks {
            let front = st.window().map(|h| areas(&w.front_of(h))).unwrap_or_default();
            let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (ball, c) in st.balls.iter().zip(&st.centers) {
                l = l.min(c.x - ball.r * 2.2);
                r = r.max(c.x + ball.r * 2.2);
                t = t.min(c.y - ball.r * 2.0);
                b = b.max(c.y + ball.r * 1.9);
            }
            if l < r {
                self.occluded(Area { l, t, r, b }, &front, |rr| rr.stack(st));
            }
        }
        if let Some(c) = &w.carried {
            self.stack(&c.stack);
        }
        if let Some(b) = &w.firebox {
            self.firebox(w, b);
        }
        for p in &w.plows {
            self.plow(w, p);
        }

        let (flake, ball) = (self.ids.flake, self.ids.ball);
        for p in &w.particles {
            match p.kind {
                Kind::Flake => self.sprite(flake, p.pos, p.r * 1.9, p.alpha),
                Kind::Chunk => self.sprite(ball, p.pos, p.r, 1.0),
                Kind::Spray => self.sprite(flake, p.pos, p.r * 2.0, p.alpha.min(1.0)),
            }
        }
        for b in &w.balls {
            self.sprite(ball, b.pos, b.r, 1.0);
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
                    self.sprite(ball, h.pos + o * (r * 0.9 * spread), r * lerp(sz, 1.0, c), 1.0);
                }
            } else {
                self.sprite(ball, h.pos, r, 1.0);
            }
            self.speckles(h.pos, r, h.rot, ((c - 0.4) * 1.5).clamp(0.0, 0.35));
        }

        self.fireworks(w);

        if let Some(b) = &w.banner {
            let a = b.alpha.clamp(0.0, 1.0);
            self.text(&b.text, b.center, 120.0 * s, rgba(1.0, 0.97, 0.85, a), 4.0 * s);
            if let Some(sub) = &b.sub {
                self.text(sub, b.center + V2(0.0, 110.0 * s), 56.0 * s, rgba(1.0, 0.85, 0.35, a), 3.0 * s);
            }
        }
    }

    /// Drift fill and outline straight from the heightmaps, clipped by windows in front.
    fn drifts(&mut self, w: &World) {
        let pile = rgba(0.975, 0.985, 1.0, 1.0);
        let edge = rgba(0.55, 0.64, 0.78, 0.75);
        let lw = (1.0f32).max(1.0 / self.max_scale) * w.scale.max(1.0);
        for surf in w.surfaces() {
            if surf.hidden || surf.heights.is_empty() {
                continue;
            }
            let n = surf.heights.len();
            let (left, top) = (surf.left as f32, surf.top as f32);
            let area = Area { l: left - 2.0, t: top - surf.max_h * 1.7 - 4.0, r: left + n as f32 + 2.0, b: top };
            let front = if surf.key < (1u64 << 62) { areas(&w.front_of(surf.key as isize)) } else { Vec::new() };
            let hs = &surf.heights;
            let (join_l, join_r) = (surf.join_l, surf.join_r);
            self.occluded(area, &front, |r| {
                let mut i = 0;
                while i < n {
                    if hs[i] < 0.3 {
                        i += 1;
                        continue;
                    }
                    let a = i;
                    while i < n && hs[i] >= 0.3 {
                        i += 1;
                    }
                    let b = i;
                    let mut pts: Vec<V2> = (a..b).step_by(2).map(|c| V2(left + c as f32 + 0.5, top - hs[c])).collect();
                    pts.push(V2(left + b as f32 - 0.5, top - hs[b - 1]));
                    // fill: a strip of trapezoids down to the base line
                    let x0 = left + a as f32;
                    let x1 = left + b as f32;
                    let first = V2(x0, pts[0].y);
                    let last = V2(x1, pts[pts.len() - 1].y);
                    let mut prev = first;
                    for &p in pts.iter().chain(std::iter::once(&last)) {
                        if p.x > prev.x {
                            r.tri(V2(prev.x, top), prev, p, pile);
                            r.tri(V2(prev.x, top), p, V2(p.x, top), pile);
                        }
                        prev = p;
                    }
                    // outline along the top, with vertical ends only where the drift really ends
                    let edge_l = a > 0 || !join_l;
                    let edge_r = b < n || !join_r;
                    if edge_l {
                        r.line(V2(x0, top), first, lw, edge);
                    }
                    let mut prev = first;
                    for &p in pts.iter().chain(std::iter::once(&last)) {
                        r.line(prev, p, lw, edge);
                        prev = p;
                    }
                    if edge_r {
                        r.line(last, V2(x1, top), lw, edge);
                    }
                }
            });
        }
    }

    fn stack(&mut self, st: &Stack) {
        let ids = (self.ids.arm, self.ids.ball, self.ids.nose);
        let snowman = st.is_snowman() && st.centers.len() >= 3;
        if snowman {
            let body = st.centers[1];
            let rb = st.balls[1].r;
            let a = rb * 1.45;
            let ah = a * self.aspect(ids.0);
            self.image(ids.0, body.x + rb * 0.62, body.y + rb * 0.2 - ah, a, ah, false);
            self.image(ids.0, body.x - rb * 0.62 - a, body.y + rb * 0.2 - ah, a, ah, true);
        }
        for (b, c) in st.balls.iter().zip(&st.centers) {
            self.sprite(ids.1, *c, b.r, 1.0);
            self.speckles(*c, b.r, b.rot, 0.25);
        }
        if !snowman {
            return;
        }
        let body = st.centers[1];
        let rb = st.balls[1].r;
        let head = st.centers[2];
        let rh = st.balls[2].r;
        let coal = rgba(0.12, 0.12, 0.15, 1.0);
        for k in [-0.42f32, 0.02, 0.46] {
            let c = body + V2(0.0, rb * k);
            self.circle(c, (rb * 0.085).max(1.5), coal);
            self.circle(c + V2(-rb * 0.025, -rb * 0.025), (rb * 0.03).max(0.6), rgba(0.45, 0.45, 0.5, 1.0));
        }
        let look = st.look;
        let scarf = self.ids.scarves[(look / 4) as usize % self.ids.scarves.len()];
        let sw = rh * 2.3;
        let sh = sw * self.aspect(scarf);
        self.image(scarf, head.x - sw * 0.5, head.y + rh * 0.62, sw, sh, false);
        let face = self.ids.faces[(look / 12) as usize % self.ids.faces.len()];
        let fw = rh * 1.1;
        let fh = fw * self.aspect(face);
        self.image(face, head.x - fw * 0.5, head.y - rh * 0.45, fw, fh, false);
        let nw = rh * 0.8;
        let nh = nw * self.aspect(ids.2);
        self.image(ids.2, head.x - rh * 0.05, head.y - nh * 0.35, nw, nh, false);
        // (sprite, width in head radii, how far down onto the head it sits)
        let fits = [(2.05, 0.55), (2.0, 0.35), (1.9, 0.38), (1.95, 0.5)];
        let k = look as usize % self.ids.hats.len();
        let (wk, sink) = fits[k];
        let hat = self.ids.hats[k];
        let hw = rh * wk;
        let hh = hw * self.aspect(hat);
        self.image(hat, head.x - hw * 0.5, head.y - rh * sink - hh, hw, hh, false);
    }

    fn firebox(&mut self, w: &World, b: &FireBox) {
        let s = w.scale;
        let (x, y, bw, bh) = (b.pos.x, b.pos.y, b.w, b.h);
        let tex = self.ids.firebox;
        let iw = bw * 1.08;
        let ih = iw * self.aspect(tex);
        self.image(tex, x - iw / 2.0, y - ih, iw, ih, false);
        if w.fireworks.active() {
            let p = V2(x + bw * 0.28, y - bh - 2.0 * s);
            let g = self.ids.glow[1];
            self.sprite(g, p, (6.0 + 3.0 * (w.time * 25.0).sin()) * s, 1.0);
        }
        if let Some(label) = &b.label {
            self.text(label, V2(x, y - ih - 14.0 * s), 12.0 * s, rgba(1.0, 0.95, 0.8, 0.95), 1.2 * s);
        }
    }

    fn plow(&mut self, w: &World, p: &Plow) {
        let s = w.scale;
        let Some(f) = w.floors.get(p.floor) else { return };
        let base = f.top as f32;
        let d = p.dir;
        let tex = self.ids.plow;
        let pw = 2.0 * (fx::PLOW_BLADE + 2.0) * s;
        let ph = pw * self.aspect(tex);
        let bounce = (p.t * 13.0).sin().abs() * 1.2 * s;
        let (l, t) = (p.x - pw / 2.0, base - ph - bounce + 2.0 * s);
        let load = (p.load.sqrt() * 0.7 * s).min(16.0 * s);
        if load > 1.0 {
            let ball = self.ids.ball;
            self.sprite(ball, V2(p.x + d * (pw / 2.0 + load * 0.4), base - load * 0.6), load, 1.0);
        }
        self.image(tex, l, t, pw, ph, d < 0.0);
        if (p.t * 7.0).sin() > 0.0 {
            let bx = p.x + d * (0.53 - 0.5) * pw;
            let g = self.ids.glow[1];
            self.blend = Blend::Add;
            self.sprite(g, V2(bx, t + ph * 0.06), 20.0 * s, 0.9);
            self.blend = Blend::Over;
        }
    }

    fn fireworks(&mut self, w: &World) {
        let fw = &w.fireworks;
        if fw.sparks.is_empty() && fw.rockets.is_empty() {
            return;
        }
        let s = w.scale;
        self.blend = Blend::Add;
        let glow = self.ids.glow.clone();
        // grouped by colour so neighbouring quads share a draw call
        for k in 0..glow.len() {
            for (i, sp) in fw.sparks.iter().enumerate() {
                if sp.color as usize % glow.len() != k {
                    continue;
                }
                let mut a = (sp.life / sp.max_life).clamp(0.0, 1.0).powf(0.7);
                if sp.twinkle && (w.time * 30.0 + i as f32 * 1.7).sin() < 0.0 {
                    a *= 0.15;
                }
                self.sprite(glow[k], sp.pos, sp.size * 2.2, a);
            }
        }
        for r in &fw.rockets {
            self.sprite(glow[7], r.pos, 4.5 * s, 1.0);
            self.sprite(glow[r.color as usize % glow.len()], r.pos, 9.0 * s, 0.5);
        }
        self.blend = Blend::Over;
    }

    // ---------------------------------------------------------------- frame

    /// Builds the frame from the world and draws it on every screen. `None` clears the screens.
    pub fn draw(&mut self, w: Option<&World>) {
        self.verts.clear();
        self.cmds.clear();
        self.blend = Blend::Over;
        self.clip = None;
        if let Some(w) = w {
            self.scene(w);
        }
        // newly rasterized glyphs
        if let Some(g) = self.glyphs.as_mut() {
            if g.dirty {
                g.dirty = false;
                let atlas = &self.textures[self.ids.atlas].texture;
                let px = std::mem::take(&mut g.pixels);
                self.upload(atlas, ATLAS, ATLAS, &px);
                if let Some(g) = self.glyphs.as_mut() {
                    g.pixels = px;
                }
            }
        }
        let bytes = (self.verts.len() * std::mem::size_of::<Vertex>()) as u64;
        if bytes > self.vbuf.size() {
            self.vbuf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: bytes.next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if bytes > 0 {
            // SAFETY: Vertex is plain old data (repr(C), only f32s)
            let raw = unsafe { std::slice::from_raw_parts(self.verts.as_ptr() as *const u8, bytes as usize) };
            self.queue.write_buffer(&self.vbuf, 0, raw);
        }
        for i in 0..self.targets.len() {
            self.present(i);
        }
    }

    fn present(&mut self, i: usize) {
        let t = &self.targets[i];
        let u = [t.origin.x, t.origin.y, t.size.x, t.size.y];
        // SAFETY: four f32s
        let raw = unsafe { std::slice::from_raw_parts(u.as_ptr() as *const u8, 16) };
        self.queue.write_buffer(&t.ubuf, 0, raw);
        let frame = match t.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => {
                // lost or outdated: configure again and skip this frame
                t.surface.configure(&self.device, &t.config);
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.msaa,
                    depth_slice: None,
                    resolve_target: Some(&view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !self.cmds.is_empty() {
                pass.set_vertex_buffer(0, self.vbuf.slice(..));
                pass.set_bind_group(0, &t.ubind, &[]);
                let (pw, ph) = (t.config.width, t.config.height);
                let mut blend = None;
                for c in &self.cmds {
                    if c.start == c.end {
                        continue;
                    }
                    // the clip in this screen's pixels
                    let (l, tp, r, b) = match c.clip {
                        Some(a) => (
                            ((a.l - t.origin.x) * t.scale).floor().max(0.0) as u32,
                            ((a.t - t.origin.y) * t.scale).floor().max(0.0) as u32,
                            (((a.r - t.origin.x) * t.scale).ceil().max(0.0) as u32).min(pw),
                            (((a.b - t.origin.y) * t.scale).ceil().max(0.0) as u32).min(ph),
                        ),
                        None => (0, 0, pw, ph),
                    };
                    if l >= r || tp >= b {
                        continue;
                    }
                    if blend != Some(c.blend) {
                        blend = Some(c.blend);
                        pass.set_pipeline(if c.blend == Blend::Add { &self.pipe_add } else { &self.pipe_over });
                    }
                    pass.set_scissor_rect(l, tp, r - l, b - tp);
                    pass.set_bind_group(1, &self.textures[c.tex].bind, &[]);
                    pass.draw(c.start..c.end, 0..1);
                }
            }
        }
        self.queue.submit([enc.finish()]);
        self.queue.present(frame);
    }
}
