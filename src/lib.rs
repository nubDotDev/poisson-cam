use std::sync::Arc;

use faster_poisson::PoissonPixelPie;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, HtmlVideoElement, MediaStreamConstraints, console, window};
use wgpu::{Adapter, Device, Queue, Surface};
use winit::{event_loop::EventLoop, window::Window};

// TODO: Delete.
#[wasm_bindgen]
extern "C" {
    pub fn alert(s: &str);
}

#[wasm_bindgen]
pub fn main() {
    wasm_bindgen_futures::spawn_local(async {
        let app = App::new().await;
        app.run();
    });
}

struct App {
    event_loop: EventLoop<()>,
    window: Arc<Window>,
    surface: Surface<'static>,
    adapter: Adapter,
    device: Arc<Device>,
    queue: Arc<Queue>,
    dims: [u16; 2],
    poisson: PoissonPixelPie<Arc<Device>, Arc<Queue>>,
}

impl App {
    async fn new() -> App {
        use winit::platform::web::WindowBuilderExtWebSys;

        let window_js = window().unwrap_throw();
        let document = window_js.document().unwrap_throw();

        let webcam: HtmlVideoElement = document
            .get_element_by_id("webcam")
            .unwrap_throw()
            .dyn_into()
            .unwrap_throw();
        let canvas: HtmlCanvasElement = document
            .get_element_by_id("canvas")
            .unwrap_throw()
            .dyn_into()
            .unwrap_throw();
        let dims = [canvas.width() as u16, canvas.height() as u16];
        let constraints = MediaStreamConstraints::new();
        constraints.set_video(&JsValue::TRUE);
        let callback = Closure::new(move |stream: JsValue| {
            webcam.set_src_object(Some(&stream.dyn_into().unwrap_throw()))
        });
        let _ = window_js
            .navigator()
            .media_devices()
            .unwrap_throw()
            .get_user_media_with_constraints(&constraints)
            .unwrap_throw()
            .then(&callback);
        callback.forget();

        env_logger::init();
        let event_loop = winit::event_loop::EventLoop::new().unwrap_throw();
        let builder = winit::window::WindowBuilder::new().with_canvas(Some(canvas));
        let window = Arc::new(builder.build(&event_loop).unwrap_throw());
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });
        let surface = instance.create_surface(window.clone()).unwrap_throw();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .unwrap_throw();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .unwrap_throw();
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        // TODO: Figure out dimensions.
        let config = surface
            .get_default_config(&adapter, dims[0] as u32, dims[1] as u32)
            .unwrap_throw();
        surface.configure(&device, &config);

        let poisson = PoissonPixelPie::new(device.clone(), queue.clone(), dims, 20.0, Some(1));

        App {
            event_loop,
            window,
            surface,
            adapter,
            device,
            queue,
            dims,
            poisson,
        }
    }

    fn run(self) {
        self.event_loop
            .run(move |event, target| {
                use winit::event::Event;

                if let Event::WindowEvent {
                    window_id: _,
                    event,
                } = event
                {
                    use winit::event::WindowEvent;

                    match event {
                        WindowEvent::RedrawRequested => {
                            self.poisson.run();

                            let buff = self.poisson.get_points_length_ro_buffer();
                            let texture_view = self.poisson.get_depth_view(&Default::default());
                            // buff.map_async(wgpu::MapMode::Read, .., |_| {
                            let frame = self.surface.get_current_texture().unwrap_throw();
                            let blitter =
                                Plotter::new(&self.device, wgpu::TextureFormat::Bgra8Unorm);
                            let sampler = self
                                .device
                                .create_sampler(&wgpu::SamplerDescriptor::default());
                            let view = frame.texture.create_view(&Default::default());
                            let mut encoder =
                                self.device.create_command_encoder(&Default::default());
                            blitter.blit(
                                &self.device,
                                &mut encoder,
                                &view,
                                &texture_view,
                                &sampler,
                            );
                            self.queue.submit([encoder.finish()]);
                            self.window.pre_present_notify();
                            frame.present();
                            // });
                        }
                        WindowEvent::CloseRequested => target.exit(),
                        _ => {}
                    }
                }
            })
            .unwrap_throw();
    }
}

pub struct Plotter {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl Plotter {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self { pipeline, layout }
    }

    pub fn blit(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        src: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit bind group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(src),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blit pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
