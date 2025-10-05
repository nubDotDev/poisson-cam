use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex},
};

use faster_poisson::PoissonPixelPie;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Document, HtmlCanvasElement, HtmlVideoElement, MediaStreamConstraints, console, window,
};
use wgpu::{
    BindGroup, Buffer, Device, Queue, RenderPipeline, Surface, Texture, TextureFormat, TextureView,
    util::DeviceExt,
};
use winit::{event::Event, event_loop::EventLoop, platform::web, window::Window};

// TODO: Delete.
#[wasm_bindgen]
extern "C" {
    pub fn alert(s: &str);
}

#[wasm_bindgen]
pub fn main() {
    spawn_local(async {
        let app = App::new().await;
        app.run();
    });
}

struct App {
    // TODO: Remove unnecessary Arcs.
    window_js: Arc<web_sys::Window>,
    document: Document,
    event_loop: EventLoop<()>,
    window: Arc<Window>,
    surface: Arc<Surface<'static>>,
    device: Arc<Device>,
    queue: Arc<Queue>,
    poisson: PoissonPixelPie<Arc<Device>, Arc<Queue>>,
    plotter: Arc<Plotter>,
}

impl App {
    async fn new() -> App {
        use winit::platform::web::WindowBuilderExtWebSys;

        let window_js = Arc::new(window().unwrap_throw());
        let document = window_js.document().unwrap_throw();

        // Stream webcam footage to video element.
        let webcam: HtmlVideoElement = document
            .get_element_by_id("webcam")
            .unwrap_throw()
            .dyn_into()
            .unwrap_throw();
        let constraints = MediaStreamConstraints::new();
        constraints.set_video(&JsValue::TRUE);
        let stream = JsFuture::from(
            window_js
                .navigator()
                .media_devices()
                .unwrap_throw()
                .get_user_media_with_constraints(&constraints)
                .unwrap_throw(),
        )
        .await
        .unwrap_throw();
        webcam.set_src_object(Some(&stream.dyn_into().unwrap_throw()));

        // Wait for the video element's metadata to load.
        // We cannot get the video dimensions until this event is triggered.
        let (tx, rx) = flume::bounded(1);
        let closure =
            Closure::wrap(Box::new(|e: web_sys::Event| tx.send(e).unwrap()) as Box<dyn FnMut(_)>);
        webcam
            .add_event_listener_with_callback("loadedmetadata", closure.as_ref().unchecked_ref())
            .unwrap();
        rx.recv_async().await.unwrap();

        // Set canvas dimensions.
        let canvas: HtmlCanvasElement = document
            .get_element_by_id("canvas")
            .unwrap_throw()
            .dyn_into()
            .unwrap_throw();
        canvas.set_width(webcam.video_width());
        canvas.set_height(webcam.video_height());
        let dims = [canvas.width() as u16, canvas.height() as u16];

        // Set up wgpu.
        env_logger::init();
        let event_loop = winit::event_loop::EventLoop::new().unwrap_throw();
        let builder = winit::window::WindowBuilder::new().with_canvas(Some(canvas));
        let window = Arc::new(builder.build(&event_loop).unwrap_throw());
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });
        let surface = Arc::new(instance.create_surface(window.clone()).unwrap_throw());
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
        let config = surface
            .get_default_config(&adapter, dims[0] as u32, dims[1] as u32)
            .unwrap_throw();
        surface.configure(&device, &config);

        let poisson = PoissonPixelPie::new(device.clone(), queue.clone(), dims, 5.0, Some(1));
        let plotter = Arc::new(Plotter::new(
            &device,
            &poisson,
            surface.get_capabilities(&adapter).formats[0],
            0.5,
        ));

        App {
            window_js,
            document,
            event_loop,
            window,
            surface,
            device,
            queue,
            poisson,
            plotter,
        }
    }

    fn run(self) {
        let is_mapped = Arc::new(Mutex::new(false));

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
                            console::log_1(&"Redrawing!".into());

                            // self.queue.copy_external_image_to_texture(
                            //     &wgpu::CopyExternalImageSourceInfo {
                            //         source: wgpu::ExternalImageSource::HTMLVideoElement(
                            //             self.document
                            //                 .get_element_by_id("webcam")
                            //                 .unwrap_throw()
                            //                 .dyn_into()
                            //                 .unwrap_throw(),
                            //         ),
                            //         origin: wgpu::Origin2d::ZERO,
                            //         flip_y: false,
                            //     },
                            //     wgpu::wgt::CopyExternalImageDestInfo {
                            //         texture: todo!(),
                            //         mip_level: todo!(),
                            //         origin: todo!(),
                            //         aspect: todo!(),
                            //         color_space: todo!(),
                            //         premultiplied_alpha: todo!(),
                            //     },
                            //     wgpu::Extent3d {
                            //         width: todo!(),
                            //         height: todo!(),
                            //         depth_or_array_layers: todo!(),
                            //     },
                            // );

                            self.poisson.run();

                            let buff = self.poisson.get_points_length_ro_buffer().clone();
                            let mut encoder =
                                self.device.create_command_encoder(&Default::default());

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

                            let (tx, rx) = flume::bounded(1);
                            buff.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
                            self.device.poll(wgpu::PollType::Wait).unwrap();
                            spawn_local(async move {
                                rx.recv_async().await.unwrap().unwrap();
                                let points_length: u32 =
                                    bytemuck::cast_slice(&buff.get_mapped_range(..))[0];
                                buff.unmap();
                                queue.submit([]);

                                let frame = surface.get_current_texture().unwrap_throw();
                                let view = frame.texture.create_view(&Default::default());
                                plotter.plot(&mut encoder, &view, points_length);

                                queue.submit([encoder.finish()]);
                                window.pre_present_notify();
                                frame.present();

                                *is_mapped.lock().unwrap() = false;
                                console::log_1(&"unmapped".into());

                                // window.request_redraw();
                            });
                        }
                        WindowEvent::CloseRequested => target.exit(),
                        _ => {}
                    }
                }
            })
            .unwrap_throw();
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

    fn plot(&self, encoder: &mut wgpu::CommandEncoder, view: &TextureView, points_length: u32) {
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
    }

    fn get_radius_uniform(&self) -> &Buffer {
        &self.radius_uniform
    }
}

struct WebcamToRadii {
    texture: Texture,
}

impl WebcamToRadii {
    fn new(device: &Device) -> WebcamToRadii {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("webcam"),
            size: wgpu::Extent3d {
                width: todo!(),
                height: todo!(),
                depth_or_array_layers: todo!(),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: todo!(),
            usage: todo!(),
            view_formats: todo!(),
        });
    }
}
