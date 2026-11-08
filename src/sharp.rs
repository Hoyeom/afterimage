//! Magnified images, drawn by our own shader (`sharp.wgsl`).
//!
//! Nearest filtering makes pixel art uneven at fractional zooms and linear
//! filtering smears it. The shader keeps pixels square at any zoom and blends
//! only the screen pixel a seam falls on. egui can't do this per texture, so
//! this reads the texture egui already uploaded; no extra memory per image.

use eframe::egui::{Painter, Rect, TextureId};
use eframe::egui_wgpu::{
    self, wgpu, CallbackResources, CallbackTrait, RenderState, ScreenDescriptor,
};

const LOCALS_SIZE: u64 = 32;

/// Draw `tex` into `rect` if it's magnified. Returns false when the caller
/// should draw it the normal way: at or below 100%, or before the first upload.
pub fn paint(
    painter: &Painter,
    state: &RenderState,
    tex: TextureId,
    rect: Rect,
    ppp: f32,
    alpha: f32,
) -> bool {
    let Some(texture) = state
        .renderer
        .read()
        .texture(&tex)
        .and_then(|t| t.texture.clone())
    else {
        return false;
    };
    if rect.width() * ppp <= texture.width() as f32 + 0.5 {
        return false;
    }
    let visible = rect.expand(1.0).intersect(painter.clip_rect());
    if visible.is_positive() {
        painter.add(egui_wgpu::Callback::new_paint_callback(
            visible,
            Draw {
                texture,
                // A fractional origin would put a seam through every pixel at
                // whole-number zooms.
                min: [(rect.min.x * ppp).round(), (rect.min.y * ppp).round()],
                size: [rect.width() * ppp, rect.height() * ppp],
                alpha,
                format: state.target_format,
            },
        ));
    }
    true
}

struct Draw {
    texture: wgpu::Texture,
    min: [f32; 2],
    size: [f32; 2],
    alpha: f32,
    format: wgpu::TextureFormat,
}

/// Built on first use, so opening an image that fits costs nothing.
struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    locals: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
}

impl Resources {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::include_wgsl!("sharp.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sharp"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(LOCALS_SIZE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sharp"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sharp"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sharp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let locals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sharp"),
            size: LOCALS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            sampler,
            locals,
            bind_group: None,
        }
    }
}

impl CallbackTrait for Draw {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if !resources.contains::<Resources>() {
            resources.insert(Resources::new(device, self.format));
        }
        let res = resources.get_mut::<Resources>().unwrap();

        let mut bytes = Vec::with_capacity(LOCALS_SIZE as usize);
        for v in [
            self.min[0],
            self.min[1],
            self.size[0],
            self.size[1],
            self.alpha,
        ] {
            bytes.extend_from_slice(&v.to_ne_bytes());
        }
        bytes.extend_from_slice(&u32::from(self.format.is_srgb()).to_ne_bytes());
        bytes.resize(LOCALS_SIZE as usize, 0);
        queue.write_buffer(&res.locals, 0, &bytes);

        let view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        res.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sharp"),
            layout: &res.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: res.locals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&res.sampler),
                },
            ],
        }));
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(res) = resources.get::<Resources>() else {
            return;
        };
        let Some(bind_group) = &res.bind_group else {
            return;
        };
        pass.set_pipeline(&res.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
