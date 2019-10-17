use crate::*;

use na::{Matrix4, Point3, Vector3};

use heightmap_gpu::HeightmapGpu;
use imgui::*;
use imgui_wgpu::Renderer;
use imgui_winit_support;
use imgui_winit_support::WinitPlatform;
use model_gpu::ModelGpu;
use std::time::Instant;
use wgpu::{BufferMapAsyncResult, Extent3d, SwapChain, TextureFormat};

use log::info;

use winit::event::WindowEvent;

struct ImguiWrap {
    imgui: imgui::Context,
    platform: WinitPlatform,
    renderer: Renderer,
}

pub struct App {
    //Wgpu
    sc_desc: wgpu::SwapChainDescriptor,
    device: wgpu::Device,
    window: winit::window::Window,
    hidpi_factor: f64,
    swap_chain: SwapChain,
    surface: wgpu::Surface,
    //Physics
    phy_state: phy_state::State,

    forward_depth: wgpu::TextureView,
    position_att: wgpu::Texture,
    position_att_view: wgpu::TextureView,
    heightmap_gpu: HeightmapGpu,
    cube_gpu: ModelGpu,
    mobile_gpu: ModelGpu,

    bind_group: wgpu::BindGroup,

    ub_camera_mat: wgpu::Buffer,
    ub_misc: wgpu::Buffer,

    postfx: post_fx::PostFx,

    frame_count: u32,
    game_state: game_state::State,
    input_state: input_state::InputState,
    imgui_wrap: ImguiWrap,

    sender_to_app: crossbeam_channel::Sender<AppMsg>,
    receiver_to_app: crossbeam_channel::Receiver<AppMsg>,

    sender_to_event_loop: crossbeam_channel::Sender<EventLoopMsg>,
}

impl App {
    pub fn new(
        window: winit::window::Window,
        sender_to_app: crossbeam_channel::Sender<AppMsg>,
        receiver_to_app: crossbeam_channel::Receiver<AppMsg>,
        sender_to_event_loop: crossbeam_channel::Sender<EventLoopMsg>,
    ) -> (Self) {
        log::trace!("App init");

        let (instance, hidpi_factor, size, surface) = {
            let instance = wgpu::Instance::new();

            window.set_inner_size(winit::dpi::LogicalSize {
                width: 1280.0,
                height: 720.0,
            });
            window.set_title("Oxidator");
            let hidpi_factor = window.hidpi_factor();
            let size = window.inner_size().to_physical(hidpi_factor);

            use raw_window_handle::HasRawWindowHandle as _;
            let surface = instance.create_surface(window.raw_window_handle());

            (instance, hidpi_factor, size, surface)
        };

        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
        });

        let mut device: wgpu::Device = adapter.request_device(&wgpu::DeviceDescriptor {
            extensions: wgpu::Extensions {
                anisotropic_filtering: false,
            },
            limits: wgpu::Limits::default(),
        });

        let sc_desc = wgpu::SwapChainDescriptor {
            usage: wgpu::TextureUsage::OUTPUT_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            width: size.width.round() as u32,
            height: size.height.round() as u32,
            present_mode: wgpu::PresentMode::NoVsync,
        };
        let swap_chain = device.create_swap_chain(&surface, &sc_desc);

        let mut init_encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { todo: 0 });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            bindings: &[
                wgpu::BindGroupLayoutBinding {
                    binding: 0,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: wgpu::BindingType::UniformBuffer { dynamic: false },
                },
                wgpu::BindGroupLayoutBinding {
                    binding: 1,
                    visibility: wgpu::ShaderStage::FRAGMENT,
                    ty: wgpu::BindingType::SampledTexture {
                        multisampled: false,
                        dimension: wgpu::TextureViewDimension::D2,
                    },
                },
                wgpu::BindGroupLayoutBinding {
                    binding: 2,
                    visibility: wgpu::ShaderStage::FRAGMENT,
                    ty: wgpu::BindingType::Sampler,
                },
                wgpu::BindGroupLayoutBinding {
                    binding: 3,
                    visibility: wgpu::ShaderStage::FRAGMENT,
                    ty: wgpu::BindingType::UniformBuffer { dynamic: false },
                },
            ],
        });

        // Create the texture
        let size = 256u32;
        let texels = fake_texels::create_texels(size as usize);
        let texture_extent = wgpu::Extent3d {
            width: size,
            height: size,
            depth: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: texture_extent,
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsage::SAMPLED | wgpu::TextureUsage::COPY_DST,
        });

        let texture_view = texture.create_default_view();
        let temp_buf = device
            .create_buffer_mapped(texels.len(), wgpu::BufferUsage::COPY_SRC)
            .fill_from_slice(&texels);
        init_encoder.copy_buffer_to_texture(
            wgpu::BufferCopyView {
                buffer: &temp_buf,
                offset: 0,
                row_pitch: 4 * size,
                image_height: size,
            },
            wgpu::TextureCopyView {
                texture: &texture,
                mip_level: 0,
                array_layer: 0,
                origin: wgpu::Origin3d {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            texture_extent,
        );

        // Create other resources
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::MirrorRepeat,
            address_mode_v: wgpu::AddressMode::MirrorRepeat,
            address_mode_w: wgpu::AddressMode::MirrorRepeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            lod_min_clamp: -100.0,
            lod_max_clamp: 100.0,
            compare_function: wgpu::CompareFunction::Always,
        });
        let mx_total = camera::create_view_proj(
            sc_desc.width as f32 / sc_desc.height as f32,
            &Point3::new(0.0, 0.0, 0.0),
            &Vector3::new(0.0, 0.0, 0.0),
        );
        let mx_ref: &[f32] = mx_total.as_slice();
        let ub_camera_mat = device
            .create_buffer_mapped(16, wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::COPY_DST)
            .fill_from_slice(mx_ref);

        let ub_misc = device
            .create_buffer_mapped(2, wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::COPY_DST)
            .fill_from_slice(&[0.0_f32, 0.0]);

        // Create bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            bindings: &[
                wgpu::Binding {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &ub_camera_mat,
                        range: 0..64,
                    },
                },
                wgpu::Binding {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::Binding {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::Binding {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer {
                        buffer: &ub_misc,
                        range: 0..8,
                    },
                },
            ],
        });

        log::trace!("   imgui_wrap init");
        let imgui_wrap = {
            // imgui
            let mut imgui = imgui::Context::create();
            let mut platform = imgui_winit_support::WinitPlatform::init(&mut imgui);
            platform.attach_window(
                imgui.io_mut(),
                &window,
                imgui_winit_support::HiDpiMode::Default,
            );
            imgui.set_ini_filename(None);

            let font_size = (13.0 * hidpi_factor) as f32;
            imgui.io_mut().font_global_scale = (1.0 / hidpi_factor) as f32;

            imgui.fonts().add_font(&[FontSource::DefaultFontData {
                config: Some(imgui::FontConfig {
                    oversample_h: 1,
                    pixel_snap_h: true,
                    size_pixels: font_size,
                    ..Default::default()
                }),
            }]);

            // imgui <-> wgpu
            let renderer = Renderer::new(&mut imgui, &mut device, sc_desc.format, None);

            ImguiWrap {
                imgui,
                platform,
                renderer,
            }
        };

        let format: TextureFormat = sc_desc.format;

        let heightmap_gpu = HeightmapGpu::new(
            &device,
            &mut init_encoder,
            format,
            &bind_group_layout,
            2048,
            2048,
        );

        let cube_gpu = ModelGpu::new(&model::create_cube(), &device, format, &bind_group_layout);

        let mobile_gpu = ModelGpu::new(&model::create_cube(), &device, format, &bind_group_layout);

        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            size: wgpu::Extent3d {
                width: sc_desc.width,
                height: sc_desc.height,
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsage::OUTPUT_ATTACHMENT,
        });

        let position_att = device.create_texture(&wgpu::TextureDescriptor {
            size: wgpu::Extent3d {
                width: sc_desc.width,
                height: sc_desc.height,
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsage::OUTPUT_ATTACHMENT | wgpu::TextureUsage::SAMPLED,
        });

        let postfx = post_fx::PostFx::new(&device, &bind_group_layout, format);

        device.get_queue().submit(&[init_encoder.finish()]);

        let mut game_state = game_state::State::new();

        let mut mobiles = Vec::new();
        for i in (100..2000).step_by(20) {
            for j in (100..2000).step_by(20) {
                mobiles.push(mobile::Mobile::new(Point3::new(i as f32, j as f32, 100.0)));
            }
        }

        println!("Number of mobiles {}", mobiles.len());

        game_state.mobiles = mobiles;

        // Done
        let this = App {
            sc_desc,
            device,
            window,
            hidpi_factor,
            swap_chain,
            surface,

            phy_state: phy_state::State::new(),

            bind_group,
            ub_camera_mat,
            ub_misc,
            cube_gpu,
            mobile_gpu,
            heightmap_gpu,
            forward_depth: depth_texture.create_default_view(),
            position_att_view: position_att.create_default_view(),
            position_att,

            postfx,

            game_state,
            input_state: input_state::InputState::new(),
            imgui_wrap,
            frame_count: 0,

            sender_to_app,
            receiver_to_app,
            sender_to_event_loop,
        };

        (this)
    }

    fn resize(&mut self) -> Option<wgpu::CommandBuffer> {
        log::trace!("resize");
        let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            size: wgpu::Extent3d {
                width: self.sc_desc.width,
                height: self.sc_desc.height,
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsage::OUTPUT_ATTACHMENT,
        });
        self.forward_depth = depth_texture.create_default_view();

        let position_att = self.device.create_texture(&wgpu::TextureDescriptor {
            size: wgpu::Extent3d {
                width: self.sc_desc.width,
                height: self.sc_desc.height,
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsage::OUTPUT_ATTACHMENT | wgpu::TextureUsage::SAMPLED,
        });

        self.position_att_view = position_att.create_default_view();
        self.position_att = position_att;

        None
    }

    pub fn update(&mut self, _event: &winit::event::Event<()>) {
        log::trace!("[app.rs] update {:?}", _event);
        use winit::event;

        self.imgui_wrap
            .platform
            .handle_event(self.imgui_wrap.imgui.io_mut(), &self.window, _event);

        //Low level
        match _event {
            event::Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                let physical = size.to_physical(self.hidpi_factor);
                info!("Resizing to {:?}", physical);
                self.sc_desc.width = physical.width.round() as u32;
                self.sc_desc.height = physical.height.round() as u32;
                self.swap_chain = self.device.create_swap_chain(&self.surface, &self.sc_desc);
                let command_buf = self.resize();
                if let Some(command_buf) = command_buf {
                    self.device.get_queue().submit(&[command_buf]);
                }
            }
            event::Event::WindowEvent { event, .. } => match event {
                WindowEvent::KeyboardInput {
                    input:
                        event::KeyboardInput {
                            virtual_keycode: Some(event::VirtualKeyCode::Escape),
                            state: event::ElementState::Pressed,
                            ..
                        },
                    ..
                }
                | WindowEvent::CloseRequested => {
                    self.sender_to_event_loop.send(EventLoopMsg::Stop).unwrap();
                }
                WindowEvent::KeyboardInput {
                    input:
                        event::KeyboardInput {
                            virtual_keycode: Some(vkc),
                            state: event::ElementState::Pressed,
                            ..
                        },
                    ..
                } => {
                    self.input_state.key_pressed.insert(vkc.clone());
                }
                WindowEvent::KeyboardInput {
                    input:
                        event::KeyboardInput {
                            virtual_keycode: Some(vkc),
                            state: event::ElementState::Released,
                            ..
                        },
                    ..
                } => {
                    self.input_state.key_pressed.remove(vkc);
                }

                WindowEvent::MouseWheel {
                    delta: event::MouseScrollDelta::LineDelta(_, dy),
                    ..
                } => {
                    self.input_state.last_scroll = *dy;
                }

                WindowEvent::CursorMoved { position, .. } => {
                    self.input_state.cursor_pos = (position.x as u32, position.y as u32)
                }

                WindowEvent::MouseInput { state, button, .. } => {
                    if !self.imgui_wrap.imgui.io().want_capture_mouse {
                        if let &winit::event::ElementState::Pressed = state {
                            self.input_state.mouse_pressed.insert(*button);
                        } else {
                            self.input_state.mouse_pressed.remove(button);
                        }
                    }
                }

                _ => {}
            },
            event::Event::EventsCleared => {
                //                self.render();
            }
            _ => (),
        }
    }

    fn render(&mut self) {
        log::trace!("render");

        self.phy_state.step();

        let frame = &self.swap_chain.get_next_texture();

        let mut now = Instant::now();
        let mut delta = now - self.game_state.last_frame;
        let last_compute_time = delta.clone();

        //empiric, a feed back loop could find this value automatically
        let oversleep = 60;
        let min_us = 1000000_u64 / self.input_state.fps;
        let min_wait_until_next_frame = std::time::Duration::from_micros(min_us - oversleep);
        if min_wait_until_next_frame > delta {
            std::thread::sleep(min_wait_until_next_frame - delta);
        }

        now = Instant::now();
        delta = now - self.game_state.last_frame;
        self.game_state.last_frame = now;
        let last_compute_time_total = delta.clone();

        let delta_sim_sec = last_compute_time_total.as_secs_f32();

        self.frame_count += 1;

        // Movements
        {
            use winit::event::VirtualKeyCode as Key;
            let key_pressed = &self.input_state.key_pressed;
            let on = |vkc| key_pressed.contains(&vkc);

            let mut offset = Vector3::new(0.0, 0.0, 0.0);
            let mut rotation = self.game_state.dir.clone();

            let camera_ground_height = self.heightmap_gpu.get_z(
                self.game_state
                    .position
                    .x
                    .max(0.0)
                    .min(self.heightmap_gpu.width as f32 - 1.0),
                self.game_state
                    .position
                    .y
                    .max(0.0)
                    .min(self.heightmap_gpu.height as f32 - 1.0),
            );
            let height_from_ground = self.game_state.position.z - camera_ground_height;
            let k = (if !on(Key::LShift) { 1.0 } else { 2.0 }) * height_from_ground.max(10.0);
            //Game
            if on(Key::S) {
                offset.y -= k;
            }
            if on(Key::Z) {
                offset.y += k;
            }
            if on(Key::Q) {
                offset.x -= k;
            }
            if on(Key::D) {
                offset.x += k;
            }

            if on(Key::LControl) {
                if self.input_state.last_scroll > 0.0 {
                    rotation.y += 1.0
                }
                if self.input_state.last_scroll < 0.0 {
                    rotation.z -= 1.0
                }
            } else {
                offset.z = -self.input_state.last_scroll * k * 20.0;
            }

            self.input_state.last_scroll = 0.0;

            self.game_state.position += offset * delta_sim_sec;
            self.game_state.dir =
                (self.game_state.dir + rotation * 33.0 * delta_sim_sec).normalize();

            self.game_state.position.z = self.game_state.position.z.max(camera_ground_height + 3.0);

            self.game_state.position_smooth += (self.game_state.position.coords
                - self.game_state.position_smooth.coords)
                * delta_sim_sec.min(0.033)
                * 15.0;

            self.game_state.dir_smooth += (self.game_state.dir - self.game_state.dir_smooth)
                * delta_sim_sec.min(0.033)
                * 15.0;
        }

        //Render
        let mut encoder_render = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { todo: 0 });

        //Phy Drawing
        {
            let cubes_t = self.phy_state.cubes_transform();
            let mut positions = Vec::with_capacity(cubes_t.len() * 16);
            for mat in cubes_t {
                positions.extend_from_slice(mat.as_slice())
            }

            self.cube_gpu.update_instance(&positions[..], &self.device);
        }

        //Mobile update

        let grid_w = (self.heightmap_gpu.width / 32) as usize;
        let grid_h = (self.heightmap_gpu.height / 32) as usize;
        let mut grid = vec![Vec::<usize>::new(); grid_w * grid_h];

        let grid_pos = |mobile: &mobile::Mobile| -> usize {
            let (x, y) = (mobile.position.x, mobile.position.y);
            (x as usize / 32) as usize + (y as usize / 32) as usize * grid_w
        };

        let mut index_to_position = Vec::with_capacity(self.game_state.mobiles.len());

        for (index, mobile) in self.game_state.mobiles.iter().enumerate() {
            index_to_position.push(mobile.position);
            grid[grid_pos(mobile)].push(index);
        }

        {
            if let Some(target) = self.game_state.mouse_world_pos {
                for (index, mobile) in self.game_state.mobiles.iter_mut().enumerate() {
                    let grid_pos = grid_pos(mobile);

                    let mut neighbors = grid[grid_pos].clone();
                    for cell in &[
                        -1_i32 - grid_w as i32,
                        -(grid_w as i32),
                        1 - grid_w as i32,
                        -1,
                        1,
                        -1 + grid_w as i32,
                        grid_w as i32,
                        1 + grid_w as i32,
                    ] {
                        let cell_index = cell + grid_pos as i32;
                        if cell_index >= 0 && (cell_index as usize) < grid_w * grid_h {
                            neighbors.extend_from_slice(&grid[cell_index as usize]);
                        }
                    }

                    let neighbors: Vec<Point3<f32>> = neighbors
                        .iter()
                        .filter(|e| **e != index)
                        .map(|index| index_to_position[*index])
                        .collect();

                    let mut dir = Vector3::new(0.0, 0.0, 0.0);

                    if neighbors.len() == 0 {
                    } else {
                        let mut nearest = neighbors.first().unwrap().clone();
                        let mut dist_min = 100000.0;
                        for neighbor in neighbors.iter() {
                            let dist = na::distance(neighbor, &mobile.position);
                            if dist_min > dist {
                                dist_min = dist;
                                nearest = neighbor.clone();
                            }
                        }

                        if dist_min < 10.0 {
                            dir = (mobile.position.coords - nearest.coords).normalize();
                            if dist_min < 3.0 {
                                dir *= 5.0;
                            }
                        }
                    }

                    let dir =
                        (dir * 2.0 + (target - mobile.position.coords).normalize()).normalize();

                    mobile.dir = mobile.dir * 0.99 + dir * 0.01;

                    mobile.speed = (mobile.speed + mobile.dir * 0.004) * 0.95;
                    mobile.position += mobile.speed;

                    mobile.position.z = self
                        .heightmap_gpu
                        .get_z_linear(mobile.position.x, mobile.position.y)
                        + 0.5;
                }
            }
        }
        {
            let cubes_t = &self.game_state.mobiles;
            let mut positions = Vec::with_capacity(cubes_t.len() * 16);
            for mobile in cubes_t {
                let mat = Matrix4::face_towards(
                    &mobile.position,
                    &(mobile.position + mobile.dir),
                    &Vector3::new(0.0, 0.0, 1.0),
                );

                positions.extend_from_slice(mat.as_slice());
            }

            self.mobile_gpu
                .update_instance(&positions[..], &self.device);
        }

        //Action

        //        if let Some(mouse_world_pos) = self.game_state.mouse_world_pos {
        //            self.game_state.heightmap_editor.handle_user_input(
        //                &self.input_state.mouse_pressed,
        //                &mouse_world_pos,
        //                &mut self.heightmap_gpu,

        //            );
        //        }

        self.heightmap_gpu.step(&self.device, &mut encoder_render);

        let cursor_sample_position = self
            .device
            .create_buffer_mapped::<f32>(
                4,
                wgpu::BufferUsage::COPY_DST | wgpu::BufferUsage::MAP_READ,
            )
            .finish(); //.fill_from_slice(initial);

        fn clamp(a: u32, min: u32, max: u32) -> u32 {
            if a < min {
                min
            } else if a > max {
                max
            } else {
                a
            }
        }

        if true || self.frame_count > 100 {
            encoder_render.copy_texture_to_buffer(
                wgpu::TextureCopyView {
                    texture: &self.position_att,
                    mip_level: 0,
                    array_layer: 0,
                    origin: wgpu::Origin3d {
                        x: clamp(self.input_state.cursor_pos.0, 0, self.sc_desc.width - 1) as f32,
                        y: clamp(self.input_state.cursor_pos.1, 0, self.sc_desc.height - 1) as f32,
                        z: 0.0,
                    },
                },
                wgpu::BufferCopyView {
                    buffer: &cursor_sample_position,
                    offset: 0,
                    row_pitch: 4 * 4,
                    image_height: 1,
                },
                Extent3d {
                    width: 1,
                    height: 1,
                    depth: 1,
                },
            );
        }

        let uniform_buf = self
            .device
            .create_buffer_mapped(2, wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::COPY_DST)
            .fill_from_slice(&[
                self.input_state.cursor_pos.0 as f32 / self.sc_desc.width as f32,
                self.input_state.cursor_pos.1 as f32 / self.sc_desc.height as f32,
            ]);

        encoder_render.copy_buffer_to_buffer(&uniform_buf, 0, &self.ub_misc, 0, 8);

        self.heightmap_gpu.update_uniform(
            &self.device,
            &mut encoder_render,
            self.game_state.position_smooth.x,
            self.game_state.position_smooth.y,
        );

        camera::update_camera_uniform(
            (self.sc_desc.width, self.sc_desc.height),
            &self.game_state.position_smooth,
            &self.game_state.dir_smooth,
            &self.ub_camera_mat,
            &self.device,
            &mut encoder_render,
        );

        //Pass
        {
            log::trace!("begin_render_pass");
            let mut rpass = encoder_render.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[
                    wgpu::RenderPassColorAttachmentDescriptor {
                        attachment: &frame.view,
                        resolve_target: None,
                        load_op: wgpu::LoadOp::Clear,
                        store_op: wgpu::StoreOp::Store,
                        clear_color: wgpu::Color {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        },
                    },
                    wgpu::RenderPassColorAttachmentDescriptor {
                        attachment: &self.position_att_view,
                        resolve_target: None,
                        load_op: wgpu::LoadOp::Clear,
                        store_op: wgpu::StoreOp::Store,
                        clear_color: wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        },
                    },
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachmentDescriptor {
                    attachment: &self.forward_depth,
                    depth_load_op: wgpu::LoadOp::Clear,
                    depth_store_op: wgpu::StoreOp::Store,
                    stencil_load_op: wgpu::LoadOp::Clear,
                    stencil_store_op: wgpu::StoreOp::Store,
                    clear_depth: 1.0,
                    clear_stencil: 0,
                }),
            });

            self.heightmap_gpu.render(&mut rpass, &self.bind_group);
            self.cube_gpu.render(&mut rpass, &self.bind_group);
            self.mobile_gpu.render(&mut rpass, &self.bind_group);
        }

        //Post pass
        {
            log::trace!("begin_post_render_pass");
            let mut rpass = encoder_render.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[wgpu::RenderPassColorAttachmentDescriptor {
                    attachment: &frame.view,
                    resolve_target: None,
                    load_op: wgpu::LoadOp::Load,
                    store_op: wgpu::StoreOp::Store,
                    clear_color: wgpu::Color {
                        r: 0.1,
                        g: 0.2,
                        b: 0.3,
                        a: 1.0,
                    },
                }],

                depth_stencil_attachment: None,
            });

            self.postfx.render(
                &mut rpass,
                &self.device,
                &self.bind_group,
                &self.position_att_view,
            );
        }

        //Imgui
        {
            log::trace!("imgui render");
            self.imgui_wrap
                .platform
                .prepare_frame(self.imgui_wrap.imgui.io_mut(), &self.window)
                .expect("Failed to prepare frame");

            let ui: Ui = self.imgui_wrap.imgui.frame();

            {
                let mut_fps = &mut self.input_state.fps;
                let debug_i1 = &mut self.input_state.debug_i1;
                let mut rebuild_heightmap = false;
                let stats_window = imgui::Window::new(im_str!("Statistics"));
                stats_window
                    .size([400.0, 200.0], imgui::Condition::FirstUseEver)
                    .position([3.0, 3.0], imgui::Condition::FirstUseEver)
                    .build(&ui, || {
                        imgui::Slider::new(im_str!("fps"), 1..=480).build(&ui, mut_fps);
                        ui.text(im_str!("Frametime: {}us", last_compute_time.as_micros()));
                        ui.text(im_str!(
                            " \" Capped: {}us",
                            last_compute_time_total.as_micros()
                        ));

                        if imgui::Slider::new(im_str!("debug_i1"), 1..=1000).build(&ui, debug_i1) {
                            rebuild_heightmap = true;
                        }
                    });

                self.game_state
                    .heightmap_editor
                    .draw_ui(&ui, &self.heightmap_gpu);

                self.phy_state.draw_ui(&ui);

                if true || rebuild_heightmap {
                    //                    let t = self.game_state.start_time.elapsed().as_secs_f32();
                }
            }
            self.imgui_wrap.platform.prepare_render(&ui, &self.window);
            self.imgui_wrap
                .renderer
                .render(ui, &self.device, &mut encoder_render, &frame.view)
                .expect("Rendering failed");
        }

        self.device.get_queue().submit(&[encoder_render.finish()]);

        let tx = self.sender_to_app.clone();
        let callback = move |e: BufferMapAsyncResult<&[f32]>| match e {
            Ok(e) => {
                log::trace!("BufferMapAsyncResult callback");
                let _ = tx.try_send(AppMsg::MapReadAsyncMessage {
                    vec: e.data.to_vec(),
                });
            }
            Err(_) => {}
        };

        cursor_sample_position.map_read_async(0, 4 * 4, callback);

        let _ = self.sender_to_app.try_send(AppMsg::Render);
    }

    pub fn map_read_async_msg(&mut self, vec: Vec<f32>) {
        if vec.len() == 4 && vec[3] != 0.0 {
            self.game_state.mouse_world_pos = Some(Vector3::new(vec[0], vec[1], vec[2]));
        } else {
            self.game_state.mouse_world_pos = None;
        }
    }

    pub fn receive(&mut self) {
        match self.receiver_to_app.try_recv() {
            Ok(x) => {
                log::trace!("receive: {:?}", x);

                match x {
                    AppMsg::EventMessage { event } => {
                        self.update(&event);
                    }
                    AppMsg::MapReadAsyncMessage { vec } => {
                        self.map_read_async_msg(vec);
                    }
                    AppMsg::Render => {
                        self.render();
                    }
                }
            }
            _ => {
                log::trace!("No message yo");
                std::thread::sleep(std::time::Duration::from_millis(1000));
            }
        }
    }
}
