mod camera;
mod lib;
mod texture;

use bytemuck::{Pod, Zeroable};
use cgmath::prelude::*;
use futures::executor::block_on;
use std::{borrow::Cow, mem};
use wgpu::util::DeviceExt;
use winit::{
    event::{DeviceEvent, ElementState, Event, VirtualKeyCode, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
};

const NUM_INSTANCES_PER_ROW: u32 = 10;

fn main() {
    let s = block_on(setup());
    start(s);
}

struct Setup {
    window: winit::window::Window,
    event_loop: EventLoop<()>,
    #[allow(dead_code)]
    instance: wgpu::Instance,
    size: winit::dpi::PhysicalSize<u32>,
    surface: wgpu::Surface,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    // #[cfg(target_arch = "wasm32")]
    // offscreen_canvas_setup: Option<OffscreenCanvasSetup>,
}

struct Scene {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: usize,
    texture_bind_group: wgpu::BindGroup,
    camera_bind_group: wgpu::BindGroup,
    camera_buf: wgpu::Buffer,
    camera_staging_buf: wgpu::Buffer,
    instances: Vec<lib::Instance>,
    instance_buf: wgpu::Buffer,
    depth_texture: texture::Texture,
    pipeline: wgpu::RenderPipeline,
    // pipeline_wire: Option<wgpu::RenderPipeline>,
}

async fn setup() -> Setup {
    let event_loop = EventLoop::new();
    let mut builder = winit::window::WindowBuilder::new();
    builder = builder.with_title("Minecrust");
    let window = builder.build(&event_loop).unwrap();

    let backend = wgpu::Backends::PRIMARY;
    let instance = wgpu::Instance::new(backend);

    let size = window.inner_size();
    let surface = unsafe {
        let surface = instance.create_surface(&window);
        surface
    };

    let adapter =
        wgpu::util::initialize_adapter_from_env_or_default(&instance, backend, Some(&surface))
            .await
            .expect("No suitable GPU adapters found on the system!");

    let adapter_info = adapter.get_info();
    println!("Using {} ({:?})", adapter_info.name, adapter_info.backend);

    let trace_dir = std::env::var("WGPU_TRACE");
    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                features: adapter.features(),
                limits: adapter.limits(),
            },
            trace_dir.ok().as_ref().map(std::path::Path::new),
        )
        .await
        .expect("Unable to find a suitable GPU adapter!");

    Setup {
        window,
        event_loop,
        instance,
        size,
        surface,
        adapter,
        device,
        queue,
    }
}

fn start(
    Setup {
        window,
        event_loop,
        instance: _,
        size,
        surface,
        adapter,
        device,
        queue,
    }: Setup,
) {
    let format = *surface
        .get_supported_formats(&adapter)
        .unwrap()
        .first()
        .unwrap();
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: format,
        width: size.width,
        height: size.height,
        present_mode: wgpu::PresentMode::Mailbox,
    };
    surface.configure(&device, &config);

    let mut camera_controller = camera::CameraController::new(0.15, 0.03);
    let mut camera = camera::Camera {
        eye: (3.0, 2.0, 3.0).into(),
        // have it look at the origin
        target: (0.0, 0.0, 0.0).into(),
        // which way is "up"
        up: cgmath::Vector3::unit_y(),
        aspect: config.width as f32 / config.height as f32,
        fovy: 45.0,
        znear: 1.0,
        zfar: 100.0,
    };
    let mut camera_uniform = camera::CameraUniform::new();
    camera_uniform.update_view_proj(&camera);

    let scene = setup_scene(&config, &adapter, &device, &queue, camera_uniform);

    let mut curr_modifier_state: winit::event::ModifiersState =
        winit::event::ModifiersState::empty();
    let mut cursor_grabbed = false;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        match event {
            Event::WindowEvent { event, window_id } => match event {
                WindowEvent::CloseRequested => {
                    if window_id == window.id() {
                        *control_flow = ControlFlow::Exit;
                    }
                }
                WindowEvent::ModifiersChanged(modifiers) => {
                    curr_modifier_state = modifiers;
                }
                WindowEvent::KeyboardInput { input, .. } => {
                    match (input.virtual_keycode, input.state) {
                        (Some(VirtualKeyCode::W), ElementState::Pressed) => {
                            if curr_modifier_state.logo() {
                                *control_flow = ControlFlow::Exit;
                                return;
                            }
                            camera_controller.process_window_event(&event);
                        }
                        _ => {
                            camera_controller.process_window_event(&event);
                        }
                    }
                }
                WindowEvent::CursorMoved { .. } => {
                    if !cursor_grabbed {
                        window.set_cursor_grab(true).expect("Failed to grab curosr");
                        window.set_cursor_visible(false);
                        cursor_grabbed = true;
                    }
                }
                _ => (),
            },

            Event::DeviceEvent { event, .. } => match event {
                DeviceEvent::MouseMotion { .. } => {
                    if cursor_grabbed {
                        camera_controller.process_device_event(&event);
                    }
                }
                _ => (),
            },

            Event::RedrawRequested(_) => {
                let frame = match surface.get_current_texture() {
                    Ok(frame) => frame,
                    Err(_) => {
                        surface.configure(&device, &config);
                        surface
                            .get_current_texture()
                            .expect("Failed to acquire next surface texture!")
                    }
                };
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                camera_controller.update_camera(&mut camera);
                camera_uniform.update_view_proj(&camera);
                queue.write_buffer(
                    &scene.camera_staging_buf,
                    0,
                    bytemuck::cast_slice(&[camera_uniform]),
                );
                render_scene(&view, &device, &queue, &scene);

                frame.present();
                camera_controller.reset_mouse_delta();
            }

            Event::MainEventsCleared => {
                // RedrawRequested will only trigger once, unless we manually
                // request it.
                window.request_redraw();
            }

            _ => (),
        }
    });
}

fn setup_scene(
    config: &wgpu::SurfaceConfiguration,
    _adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    camera_uniform: camera::CameraUniform,
) -> Scene {
    let vertex_size = mem::size_of::<Vertex>();
    let (vertex_data, index_data) = create_vertices();
    let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Vertex Buffer"),
        contents: bytemuck::cast_slice(&vertex_data),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Index Buffer"),
        contents: bytemuck::cast_slice(&index_data),
        usage: wgpu::BufferUsages::INDEX,
    });

    // Create the texture
    let texture_atlas_bytes = include_bytes!("../assets/minecruft_atlas.png");
    let texture_atlas_bytes = image::load_from_memory(texture_atlas_bytes).unwrap();
    let texture_atlas_rgba = texture_atlas_bytes.to_rgba8();
    let dimensions = texture_atlas_rgba.dimensions();

    let texture_extent = wgpu::Extent3d {
        width: dimensions.0,
        height: dimensions.1,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: texture_extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &texture_atlas_rgba,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: std::num::NonZeroU32::new(4 * dimensions.0),
            rows_per_image: std::num::NonZeroU32::new(dimensions.1),
        },
        texture_extent,
    );

    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    // Create pipeline layout
    let texture_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    // This should match the filterable field of the
                    // corresponding Texture entry above.
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
    let camera_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(64),
                },
                count: None,
            }],
        });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&texture_bind_group_layout, &camera_bind_group_layout],
        push_constant_ranges: &[],
    });

    // Camera
    let camera_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Camera Buffer"),
        contents: bytemuck::cast_slice(&[camera_uniform]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let camera_staging_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Camera Staging Buffer"),
        contents: bytemuck::cast_slice(&[camera_uniform]),
        usage: wgpu::BufferUsages::UNIFORM
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
    });

    // Create bind groups
    let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &texture_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
        label: None,
    });
    let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &camera_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_buf.as_entire_binding(),
        }],
        label: None,
    });

    let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
        label: Some("Main Shader"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("shader.wgsl"))),
    });

    let vertex_buffers = wgpu::VertexBufferLayout {
        array_stride: vertex_size as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            // position
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 0,
                shader_location: 0,
            },
            // tex_coord
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 4 * 4,
                shader_location: 1,
            },
            // texture_atlas_offset (sprite offset)
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: (4 * 4) + (2 * 4),
                shader_location: 2,
            },
        ],
    };

    let instances = (0..NUM_INSTANCES_PER_ROW)
        .flat_map(|x| {
            (0..NUM_INSTANCES_PER_ROW).map(move |z| {
                let position = cgmath::Vector3 {
                    x: (x as f32 * 2.5),
                    y: 0.0,
                    z: (z as f32 * 2.5),
                };

                // let rotation = if position.is_zero() {
                //     // this is needed so an object at (0, 0, 0) won't get scaled to zero
                //     // as Quaternions can effect scale if they're not created correctly
                //     cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(0.0))
                // } else {
                //     cgmath::Quaternion::from_axis_angle(position.normalize(), cgmath::Deg(45.0))
                // };

                // No rotation
                let rotation = cgmath::Quaternion::from_axis_angle(
                    cgmath::Vector3::unit_y(),
                    cgmath::Deg(0.0),
                );

                lib::Instance { position, rotation }
            })
        })
        .collect::<Vec<_>>();

    let instance_data = instances
        .iter()
        .map(lib::Instance::to_raw)
        .collect::<Vec<_>>();
    let instance_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Instance Buffer"),
        contents: bytemuck::cast_slice(&instance_data),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let depth_texture = texture::Texture::create_depth_texture(&device, &config, "depth_texture");

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[vertex_buffers, lib::InstanceRaw::desc()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[config.format.into()],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: texture::Texture::DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    Scene {
        vertex_buf,
        index_buf,
        index_count: index_data.len(),
        texture_bind_group,
        camera_bind_group,
        camera_buf,
        camera_staging_buf,
        instances,
        instance_buf,
        depth_texture,
        pipeline,
    }
}

fn render_scene(
    view: &wgpu::TextureView,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &Scene,
    // spawner: &framework::Spawner,
) {
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.1,
                        g: 0.2,
                        b: 0.3,
                        a: 1.0,
                    }),
                    store: true,
                },
            }],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &scene.depth_texture.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: true,
                }),
                stencil_ops: None,
            }),
        });
        rpass.push_debug_group("Prepare data for draw.");
        rpass.set_pipeline(&scene.pipeline);
        rpass.set_bind_group(0, &scene.texture_bind_group, &[]);
        rpass.set_bind_group(1, &scene.camera_bind_group, &[]);
        rpass.set_index_buffer(scene.index_buf.slice(..), wgpu::IndexFormat::Uint16);
        rpass.set_vertex_buffer(0, scene.vertex_buf.slice(..));
        rpass.set_vertex_buffer(1, scene.instance_buf.slice(..));
        rpass.pop_debug_group();
        rpass.insert_debug_marker("Draw!");

        rpass.draw_indexed(
            0..scene.index_count as u32,
            0,
            0..scene.instances.len() as _,
        );

        // TODO: wireframe
        // if let Some(ref pipe) = self.pipeline_wire {
        //     rpass.set_pipeline(pipe);
        //     rpass.draw_indexed(0..self.index_count as u32, 0, 0..1);
        // }
    }
    encoder.copy_buffer_to_buffer(
        &scene.camera_staging_buf,
        0,
        &scene.camera_buf,
        0,
        mem::size_of::<camera::CameraUniform>().try_into().unwrap(),
    );

    queue.submit(Some(encoder.finish()));

    // If an error occurs, report it and panic.
    // spawner.spawn_local(ErrorFuture {
    //     inner: device.pop_error_scope(),
    // });
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct Vertex {
    _pos: [f32; 4],
    _tex_coord: [f32; 2],
    _atlas_offset: [f32; 2],
}

fn vertex(pos: [i8; 3], tc: [i8; 2], ao: [i8; 2]) -> Vertex {
    Vertex {
        _pos: [pos[0] as f32, pos[1] as f32, pos[2] as f32, 1.0],
        _tex_coord: [tc[0] as f32, tc[1] as f32],
        _atlas_offset: [ao[0] as f32, ao[1] as f32],
    }
}

fn create_vertices() -> (Vec<Vertex>, Vec<u16>) {
    let vertex_data = [
        // top (0, 0, 1)
        vertex([-1, -1, 1], [0, 0], [1, 0]),
        vertex([1, -1, 1], [1, 0], [1, 0]),
        vertex([1, 1, 1], [1, 1], [1, 0]),
        vertex([-1, 1, 1], [0, 1], [1, 0]),
        // bottom (0, 0, -1)
        vertex([-1, 1, -1], [1, 0], [2, 0]),
        vertex([1, 1, -1], [0, 0], [2, 0]),
        vertex([1, -1, -1], [0, 1], [2, 0]),
        vertex([-1, -1, -1], [1, 1], [2, 0]),
        // right (1, 0, 0)
        vertex([1, -1, -1], [0, 0], [0, 0]),
        vertex([1, 1, -1], [1, 0], [0, 0]),
        vertex([1, 1, 1], [1, 1], [0, 0]),
        vertex([1, -1, 1], [0, 1], [0, 0]),
        // left (-1, 0, 0)
        vertex([-1, -1, 1], [1, 0], [0, 0]),
        vertex([-1, 1, 1], [0, 0], [0, 0]),
        vertex([-1, 1, -1], [0, 1], [0, 0]),
        vertex([-1, -1, -1], [1, 1], [0, 0]),
        // front (0, 1, 0)
        vertex([1, 1, -1], [1, 0], [0, 0]),
        vertex([-1, 1, -1], [0, 0], [0, 0]),
        vertex([-1, 1, 1], [0, 1], [0, 0]),
        vertex([1, 1, 1], [1, 1], [0, 0]),
        // back (0, -1, 0)
        vertex([1, -1, 1], [0, 0], [0, 0]),
        vertex([-1, -1, 1], [1, 0], [0, 0]),
        vertex([-1, -1, -1], [1, 1], [0, 0]),
        vertex([1, -1, -1], [0, 1], [0, 0]),
    ];

    let index_data: &[u16] = &[
        0, 1, 2, 2, 3, 0, // top
        4, 5, 6, 6, 7, 4, // bottom
        8, 9, 10, 10, 11, 8, // right
        12, 13, 14, 14, 15, 12, // left
        16, 17, 18, 18, 19, 16, // front
        20, 21, 22, 22, 23, 20, // back
    ];

    (vertex_data.to_vec(), index_data.to_vec())
}
