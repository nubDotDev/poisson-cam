use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex},
};

use faster_poisson::PoissonPixelPie;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Document, HtmlCanvasElement, HtmlInputElement, HtmlVideoElement, MediaStreamConstraints, window,
};
use wgpu::{
    BindGroup, Buffer, CommandEncoder, ComputePipeline, Device, Queue, RenderPipeline, Surface,
    Texture, TextureFormat, TextureView, util::DeviceExt,
};
use winit::{event::Event, event_loop::EventLoop, window::Window};

#[wasm_bindgen]
pub fn main() {
    spawn_local(async {
        let app = App::new().await;
        app.run();
    });
}

struct App {
    document: Document,
    event_loop: EventLoop<()>,
    window: Arc<Window>,
    surface: Arc<Surface<'static>>,
    device: Arc<Device>,
    queue: Arc<Queue>,
    poisson: PoissonPixelPie<Arc<Device>, Arc<Queue>>,
    plotter: Arc<Plotter>,
    webcam_to_radii: Arc<WebcamToRadii>,
}

impl App {
    async fn new() -> App {
        use winit::platform::web::WindowBuilderExtWebSys;

        let window_js = Arc::new(window().expect_throw("failed to get window"));
        let document = window_js.document().expect_throw("failed to get document");

        // Stream webcam footage to video element.
        let webcam: HtmlVideoElement = document
            .get_element_by_id("webcam")
            .expect_throw("webcam element not found")
            .dyn_into()
            .unwrap();
        let constraints = MediaStreamConstraints::new();
        constraints.set_video(&JsValue::TRUE);
        let stream = JsFuture::from(
            window_js
                .navigator()
                .media_devices()
                .expect_throw("failed to get media devices")
                .get_user_media_with_constraints(&constraints)
                .expect_throw("failed to get user media"),
        )
        .await
        .unwrap();
        webcam.set_src_object(Some(
            &stream
                .dyn_into()
                .expect_throw("failed to set webcam source"),
        ));

        // Wait for the video element's metadata to load.
        // We cannot get the video dimensions until this event is triggered.
        let (tx, rx) = flume::bounded(1);
        let closure = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
            tx.send(e).unwrap();
        });
        webcam
            .add_event_listener_with_callback("loadedmetadata", closure.as_ref().unchecked_ref())
            .expect_throw("failed to add loadedmetadata event listener");
        closure.forget();
        rx.recv_async().await.unwrap();

        // Set canvas dimensions.
        let canvas: HtmlCanvasElement = document
            .get_element_by_id("canvas")
            .expect_throw("canvas element not found")
            .dyn_into()
            .unwrap();
        canvas.set_width(webcam.video_width());
        canvas.set_height(webcam.video_height());
        let dims = [canvas.width() as u16, canvas.height() as u16];

        // Set up wgpu.
        env_logger::init();
        let event_loop =
            winit::event_loop::EventLoop::new().expect_throw("failed to make event loop");
        let builder = winit::window::WindowBuilder::new().with_canvas(Some(canvas));
        let window = Arc::new(
            builder
                .build(&event_loop)
                .expect_throw("failed to build window"),
        );
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });
        let surface = Arc::new(
            instance
                .create_surface(window.clone())
                .expect_throw("failed to create surface"),
        );
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .expect_throw("failed to get adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect_throw("failed to get device and queue");
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let config = surface
            .get_default_config(&adapter, dims[0] as u32, dims[1] as u32)
            .unwrap();
        surface.configure(&device, &config);

        let poisson = PoissonPixelPie::new(device.clone(), queue.clone(), dims, 1.0, Some(1));
        let texture_format = surface.get_capabilities(&adapter).formats[0];
        let plotter = Arc::new(Plotter::new(&device, &poisson, texture_format, 0.5));
        let webcam_to_radii = Arc::new(WebcamToRadii::new(
            &device,
            &poisson,
            [0.5, 10.0],
            RadiusMode::Shade,
            0.95,
        ));

        App {
            document,
            event_loop,
            window,
            surface,
            device,
            queue,
            poisson,
            plotter,
            webcam_to_radii,
        }
    }

    fn run(self) {
        let is_mapped = Arc::new(Mutex::new(false));
        // let mut seed = 1;

        let min_radius_slider: HtmlInputElement = self
            .document
            .get_element_by_id("min-radius")
            .expect_throw("min-radius element not found")
            .dyn_into()
            .unwrap();
        let queue = self.queue.clone();
        let webcam_to_radii = self.webcam_to_radii.clone();
        let closure = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
            queue.write_buffer(
                &webcam_to_radii.r_bounds_uniform,
                0,
                bytemuck::cast_slice(&[e
                    .current_target()
                    .unwrap()
                    .dyn_into::<HtmlInputElement>()
                    .unwrap()
                    .value_as_number() as f32]),
            );
        });
        min_radius_slider
            .add_event_listener_with_callback("input", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();

        let max_radius_slider: HtmlInputElement = self
            .document
            .get_element_by_id("max-radius")
            .expect_throw("max-radius element not found")
            .dyn_into()
            .unwrap();
        let queue = self.queue.clone();
        let webcam_to_radii = self.webcam_to_radii.clone();
        let closure = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
            queue.write_buffer(
                &webcam_to_radii.r_bounds_uniform,
                4,
                bytemuck::cast_slice(&[e
                    .current_target()
                    .unwrap()
                    .dyn_into::<HtmlInputElement>()
                    .unwrap()
                    .value_as_number() as f32]),
            );
        });
        max_radius_slider
            .add_event_listener_with_callback("input", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();

        let dot_radius_slider: HtmlInputElement = self
            .document
            .get_element_by_id("dot-radius")
            .expect_throw("dot-radius element not found")
            .dyn_into()
            .unwrap();
        let queue = self.queue.clone();
        let plotter = self.plotter.clone();
        let closure = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
            queue.write_buffer(
                &plotter.radius_uniform,
                0,
                bytemuck::cast_slice(&[e
                    .current_target()
                    .unwrap()
                    .dyn_into::<HtmlInputElement>()
                    .unwrap()
                    .value_as_number() as f32]),
            );
        });
        dot_radius_slider
            .add_event_listener_with_callback("input", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();

        let magic_slider: HtmlInputElement = self
            .document
            .get_element_by_id("magic")
            .expect_throw("magic element not found")
            .dyn_into()
            .unwrap();
        let queue = self.queue.clone();
        let webcam_to_radii = self.webcam_to_radii.clone();
        let closure = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
            queue.write_buffer(
                &webcam_to_radii.magic_uniform,
                0,
                bytemuck::cast_slice(&[e
                    .current_target()
                    .unwrap()
                    .dyn_into::<HtmlInputElement>()
                    .unwrap()
                    .value_as_number() as f32]),
            );
        });
        magic_slider
            .add_event_listener_with_callback("input", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();

        self.event_loop
            .run(move |event, target| {
                if let Event::WindowEvent {
                    window_id: _,
                    event,
                } = event
                {
                    use winit::event::WindowEvent;

                    match event {
                        WindowEvent::RedrawRequested => {
                            let mut encoder =
                                self.device.create_command_encoder(&Default::default());
                            self.queue.copy_external_image_to_texture(
                                &wgpu::CopyExternalImageSourceInfo {
                                    source: wgpu::ExternalImageSource::HTMLVideoElement(
                                        self.document
                                            .get_element_by_id("webcam")
                                            .expect_throw("webcam element not found")
                                            .dyn_into()
                                            .unwrap(),
                                    ),
                                    origin: wgpu::Origin2d::ZERO,
                                    flip_y: false,
                                },
                                wgpu::wgt::CopyExternalImageDestInfo {
                                    texture: &self.webcam_to_radii.texture,
                                    mip_level: 0,
                                    origin: wgpu::Origin3d::ZERO,
                                    aspect: wgpu::TextureAspect::All,
                                    color_space: wgpu::PredefinedColorSpace::Srgb,
                                    premultiplied_alpha: false,
                                },
                                self.webcam_to_radii.texture.size(),
                            );
                            self.webcam_to_radii.run(&mut encoder);
                            // self.poisson.set_seed(Some(seed));
                            // seed += 1;
                            self.poisson.run();

                            let window = self.window.clone();
                            let surface = self.surface.clone();
                            let queue = self.queue.clone();
                            let plotter = self.plotter.clone();

                            let mut guard = is_mapped.lock().unwrap();
                            if *guard {
                                return;
                            }
                            *guard = true;
                            let is_mapped = is_mapped.clone();

                            let buff = self.poisson.get_points_length_ro_buffer().clone();
                            let (tx, rx) = flume::bounded(1);
                            buff.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
                            self.device.poll(wgpu::PollType::Wait).unwrap();
                            spawn_local(async move {
                                rx.recv_async().await.unwrap().unwrap();
                                let points_length: u32 =
                                    bytemuck::cast_slice(&buff.get_mapped_range(..))[0];
                                buff.unmap();
                                queue.submit([]);

                                let frame = surface
                                    .get_current_texture()
                                    .expect_throw("failed to get current texture");
                                let view = frame.texture.create_view(&Default::default());
                                plotter.run(&mut encoder, &view, points_length);

                                queue.submit([encoder.finish()]);
                                window.pre_present_notify();
                                frame.present();

                                *is_mapped.lock().unwrap() = false;

                                window.request_redraw();
                            });
                        }
                        WindowEvent::CloseRequested => target.exit(),
                        _ => {}
                    }
                }
            })
            .expect_throw("event loop terminated with an error");
    }
}

struct Plotter {
    pipeline: RenderPipeline,
    bind_group: BindGroup,
    radius_uniform: Buffer,
}

impl Plotter {
    fn new(
        device: &Device,
        poisson: &PoissonPixelPie<Arc<Device>, Arc<Queue>>,
        texture_format: TextureFormat,
        radius: f32,
    ) -> Plotter {
        let module = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("plotter"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(NonZeroU64::new(8).unwrap()),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("plotter"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("plotter"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("emit_triangle"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("carve_circle"),
                targets: &[Some(texture_format.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let radius_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("radius"),
            contents: bytemuck::cast_slice(&[radius]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("plotter"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: poisson.get_dims_uniform().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: radius_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: poisson.get_points_buffer().as_entire_binding(),
                },
            ],
        });

        Plotter {
            pipeline,
            bind_group,
            radius_uniform,
        }
    }

    fn run(&self, encoder: &mut CommandEncoder, view: &TextureView, points_length: u32) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("plotter"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);

        pass.draw(0..3 * points_length, 0..1);

        drop(pass);
    }
}

struct WebcamToRadii {
    pipeline: ComputePipeline,
    bind_group: BindGroup,
    texture: Texture,
    r_bounds_uniform: Buffer,
    magic_uniform: Buffer,
    dims: [u16; 2],
}

impl WebcamToRadii {
    fn new(
        device: &Device,
        poisson: &PoissonPixelPie<Arc<Device>, Arc<Queue>>,
        r_bounds: [f32; 2],
        mode: RadiusMode,
        magic: f32,
    ) -> WebcamToRadii {
        let module = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("calc_radii"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(NonZeroU64::new(8).unwrap()),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(NonZeroU64::new(8).unwrap()),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("calc_radii"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("calc_radii"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("calc_radii"),
            compilation_options: Default::default(),
            cache: None,
        });

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("webcam"),
            size: poisson.get_depth_view(&Default::default()).texture().size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let r_bounds_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("r_bounds"),
            contents: bytemuck::cast_slice(&r_bounds),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let mode_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mode"),
            contents: bytemuck::cast_slice(&[mode as u32]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let magic_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("magic"),
            contents: bytemuck::cast_slice(&[magic]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("calc_radii"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: poisson.get_dims_uniform().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: poisson.get_radii_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: r_bounds_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: mode_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: magic_uniform.as_entire_binding(),
                },
            ],
        });

        WebcamToRadii {
            pipeline,
            bind_group,
            texture,
            r_bounds_uniform,
            magic_uniform,
            dims: poisson.get_dims(),
        }
    }

    fn run(&self, encoder: &mut CommandEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("calc_radii"),
            timestamp_writes: None,
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);

        let workgroup_count = (self.dims[0] as u32 * self.dims[1] as u32).div_ceil(64);
        pass.dispatch_workgroups(
            workgroup_count.min(32768),
            workgroup_count.div_ceil(32768),
            1,
        );

        drop(pass);
    }
}

#[derive(Clone, Copy)]
enum RadiusMode {
    Highlight,
    Shade,
    Red,
    Green,
    Blue,
}
