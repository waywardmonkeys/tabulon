// Copyright 2024 the Vello Authors
// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! DXF viewer

use anyhow::Result;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Instant;
use tracing_subscriber::prelude::*;
use vello::kurbo::{Affine, Point, Stroke, Vec2};
use vello::peniko::color::palette;
use vello::peniko::Color;
use vello::util::{RenderContext, RenderSurface};
use vello::{AaConfig, Renderer, RendererOptions, Scene};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::Window;

use vello::wgpu;

use tabulon_dxf::{EntityHandle, TDDrawing};

use tabulon::{
    render_layer::RenderLayer,
    shape::{AnyShape, FatPaint, FatShape},
    GraphicsBag, GraphicsItem,
};

extern crate alloc;

use alloc::collections::BTreeSet;

enum RenderState<'s> {
    /// `RenderSurface` and `Window` for active rendering.
    Active {
        // The `RenderSurface` and the `Window` must be in this order, so that the surface is dropped first.
        surface: Box<RenderSurface<'s>>,
        window: Arc<Window>,
    },
    /// Cache a window so that it can be reused when the app is resumed after being suspended.
    Suspended(Option<Arc<Window>>),
}

#[derive(Default)]
struct GestureState {
    /// Currently panning with primary pointer.
    primary_pan: bool,
    /// Cursor position.
    cursor_pos: Point,
}

struct SimpleVelloApp<'s> {
    /// The vello `RenderContext` which is a global context that lasts for the lifetime of the application.
    context: RenderContext,

    /// An array of renderers, one per wgpu device.
    renderers: Vec<Option<Renderer>>,

    /// The window, and also the surface while actively rendering.
    state: RenderState<'s>,

    /// A vello Scene which is a data structure which allows one to build up a description a scene to be
    /// drawn (with paths, fills, images, text, etc) which is then passed to a renderer for rendering.
    scene: Scene,

    /// `tabulon_dxf` drawing.
    drawing: TDDrawing,

    /// Index of bounding boxes for hit testing.
    picking_index: EntityIndex,
    /// Which shape is closest to the cursor?
    pick: Option<EntityHandle>,

    /// Tabulon Vello environment.
    tv_environment: tabulon_vello::Environment,

    /// View transform of the drawing.
    view_transform: Affine,
    /// View scale of the drawing.
    view_scale: f64,

    /// Defer reprojection until after redraw is completed.
    defer_reprojection: bool,

    /// State of gesture processing (e.g. panning, zooming).
    gestures: GestureState,
}

impl ApplicationHandler for SimpleVelloApp<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let RenderState::Suspended(cached_window) = &mut self.state else {
            return;
        };

        let window = cached_window
            .take()
            .unwrap_or_else(|| create_winit_window(event_loop));

        // Create a vello Surface.
        let size = window.inner_size();
        let surface_future = {
            let surface = self
                .context
                .instance
                .create_surface(wgpu::SurfaceTarget::from(window.clone()))
                .expect("Error creating surface");
            let dev_id = pollster::block_on(self.context.device(Some(&surface)))
                .expect("No compatible device");
            let device_handle = &self.context.devices[dev_id];
            let capabilities = surface.get_capabilities(device_handle.adapter());
            let present_mode = if capabilities
                .present_modes
                .contains(&wgpu::PresentMode::Mailbox)
            {
                wgpu::PresentMode::Mailbox
            } else {
                wgpu::PresentMode::AutoVsync
            };
            self.context
                .create_render_surface(surface, size.width, size.height, present_mode)
        };

        let surface = pollster::block_on(surface_future).expect("Error creating surface");

        // Create a vello Renderer for the surface (using its device id).
        self.renderers
            .resize_with(self.context.devices.len(), || None);
        self.renderers[surface.dev_id]
            .get_or_insert_with(|| create_vello_renderer(&self.context, &surface));

        // Save the Window and Surface to a state variable.
        self.state = RenderState::Active {
            surface: Box::new(surface),
            window,
        };

        let bounds = self
            .drawing
            .item_entity_map
            .iter()
            .flat_map(|(k, _v)| match self.drawing.graphics.get(*k) {
                Some(GraphicsItem::FatShape(FatShape { subshapes, .. })) => subshapes.iter(),
                _ => [].iter(),
            })
            .fold(vello::kurbo::Rect::default(), |a, x| {
                a.union(x.bounding_box())
            });

        self.view_scale = (size.height as f64 / bounds.size().height)
            .min(size.width as f64 / bounds.size().width);

        self.view_transform = Affine::translate(Vec2 {
            x: -bounds.min_x(),
            y: -bounds.min_y(),
        })
        .then_scale(self.view_scale);

        update_transform(
            &mut self.drawing.graphics,
            self.view_transform,
            self.view_scale,
        );
        self.scene.reset();

        let encode_started = Instant::now();
        self.tv_environment.add_render_layer_to_scene(
            &mut self.scene,
            &self.drawing.graphics,
            &self.drawing.render_layer,
        );
        let encode_duration = Instant::now().saturating_duration_since(encode_started);
        eprintln!("Initial projection/encode took {encode_duration:?}");
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        if let RenderState::Active { window, .. } = &self.state {
            self.state = RenderState::Suspended(Some(window.clone()));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let (surface, window) = match &mut self.state {
            RenderState::Active { surface, window } if window.id() == window_id => {
                (surface, window)
            }
            _ => return,
        };

        let mut reproject = false;
        // Set if reprojection is requested as a result of a deferral.
        let mut reproject_deferred = false;

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                use winit::keyboard::{Key, NamedKey};
                if event.state.is_pressed() && event.logical_key == Key::Named(NamedKey::Escape) {
                    event_loop.exit();
                }
            }

            WindowEvent::Resized(size) => {
                self.context
                    .resize_surface(surface, size.width, size.height);
            }

            WindowEvent::CursorMoved { position, .. } => {
                let p = {
                    let winit::dpi::PhysicalPosition::<f64> { x, y } = position;
                    Point { x, y }
                };

                let dp = self.view_transform.inverse() * p;

                if self.gestures.primary_pan {
                    self.view_transform = self
                        .view_transform
                        .then_translate(-(self.gestures.cursor_pos - p));
                    reproject = true;
                } else {
                    const PICK_DIST: f64 = 4.;
                    let pick = self
                        .picking_index
                        .pick(dp, PICK_DIST * self.view_scale.recip());

                    let pick_started = Instant::now();

                    if self.pick != pick {
                        if let Some(pick) = pick {
                            let pick_duration =
                                Instant::now().saturating_duration_since(pick_started);
                            eprintln!("{:#?}", self.drawing.info.get_entity(pick).specific);
                            eprintln!("Pick took {pick_duration:?}");
                        }
                        self.pick = pick;
                        reproject = true;
                    }
                }
                self.gestures.cursor_pos = p;
            }

            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Left,
                ..
            } => {
                self.gestures.primary_pan = matches!(state, winit::event::ElementState::Pressed);
            }

            WindowEvent::MouseWheel { delta, .. } => {
                use winit::{dpi::PhysicalPosition, event::MouseScrollDelta::*};
                let d = match delta {
                    LineDelta(_, y) => y as f64 * 0.1,
                    PixelDelta(PhysicalPosition::<f64> { y, .. }) => y * 0.05,
                };

                self.view_transform = self
                    .view_transform
                    .then_translate(-self.gestures.cursor_pos.to_vec2())
                    .then_scale(1. + d)
                    .then_translate(self.gestures.cursor_pos.to_vec2());
                self.view_scale *= 1. + d;
                reproject = true;
            }

            WindowEvent::PinchGesture { delta: d, .. } => {
                self.view_transform = self
                    .view_transform
                    .then_translate(-self.gestures.cursor_pos.to_vec2())
                    .then_scale(1. + d)
                    .then_translate(self.gestures.cursor_pos.to_vec2());
                self.view_scale *= 1. + d;
                reproject = true;
            }

            WindowEvent::RedrawRequested => {
                let wgpu::SurfaceConfiguration { width, height, .. } = surface.config;

                let device_handle = &self.context.devices[surface.dev_id];

                let surface_texture = surface
                    .surface
                    .get_current_texture()
                    .expect("failed to get surface texture");

                // Render to the surface's texture
                self.renderers[surface.dev_id]
                    .as_mut()
                    .unwrap()
                    .render_to_surface(
                        &device_handle.device,
                        &device_handle.queue,
                        &self.scene,
                        &surface_texture,
                        &vello::RenderParams {
                            base_color: Color::WHITE, // Background color
                            width,
                            height,
                            antialiasing_method: AaConfig::Area,
                        },
                    )
                    .expect("failed to render to surface");

                surface_texture.present();

                #[cfg(feature = "tracing-tracy")]
                tracy_client::frame_mark();

                device_handle.device.poll(wgpu::Maintain::Poll);

                if self.defer_reprojection {
                    reproject_deferred = true;
                    self.defer_reprojection = false;
                }
            }
            _ => {}
        }

        if reproject || reproject_deferred {
            if self.defer_reprojection {
                return;
            }
            // direct requests for reprojection until after the next redraw is complete.
            self.defer_reprojection = reproject;
            let reproject_started = Instant::now();
            update_transform(
                &mut self.drawing.graphics,
                self.view_transform,
                self.view_scale,
            );

            let tl = self.view_transform.inverse() * Point { x: 0., y: 0. };
            let br = self.view_transform.inverse()
                * Point {
                    x: surface.config.width as f64,
                    y: surface.config.height as f64,
                };
            let visible = self.picking_index.query(tl.x, tl.y, br.x, br.y);
            let culled_render_layer = self.drawing.render_layer.filter(|ih| {
                // TODO: add functionality to measure text and include it in the culling pass.
                !matches!(
                    self.drawing.graphics.get(*ih),
                    Some(GraphicsItem::FatShape(..))
                ) || visible.contains(&self.drawing.item_entity_map[ih])
            });
            self.scene.reset();
            self.tv_environment.add_render_layer_to_scene(
                &mut self.scene,
                &self.drawing.graphics,
                &culled_render_layer,
            );

            if let Some(pick) = self.pick {
                let mut gb = GraphicsBag::default();
                let mut rl = RenderLayer::default();

                gb.update_transform(Default::default(), self.view_transform);

                let paint = gb.register_paint(FatPaint {
                    stroke: Stroke::new(1.414 / self.view_scale),
                    stroke_paint: Some(palette::css::GOLDENROD.into()),
                    fill_paint: None,
                });

                self.drawing
                    .item_entity_map
                    .iter()
                    .filter(|(_ih, eh)| **eh == pick)
                    .for_each(|(ih, _eh)| {
                        let Some(GraphicsItem::FatShape(FatShape {
                            transform,
                            subshapes,
                            ..
                        })) = self.drawing.graphics.get(*ih)
                        else {
                            return;
                        };

                        rl.push_with_bag(
                            &mut gb,
                            FatShape {
                                transform: *transform,
                                subshapes: subshapes.clone(),
                                paint,
                            },
                        );
                    });

                self.tv_environment
                    .add_render_layer_to_scene(&mut self.scene, &gb, &rl);
            }

            let reproject_duration = Instant::now().saturating_duration_since(reproject_started);
            eprintln!("Reprojection/reencoding took {reproject_duration:?}");

            window.request_redraw();
        }
    }
}

fn main() -> Result<()> {
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing::level_filters::LevelFilter::WARN.into())
                .from_env_lossy(),
        );

    #[cfg(feature = "tracing-tracy")]
    let tracy_layer = tracing_tracy::TracyLayer::default();
    #[cfg(feature = "tracing-tracy")]
    let subscriber = subscriber.with(tracy_layer);

    subscriber.init();

    let drawing_load_started = Instant::now();
    let drawing = tabulon_dxf::load_file_default_layers(
        std::env::args()
            .next_back()
            .expect("Please provide a path in the last argument."),
    )
    .expect("DXF file failed to load.");

    let drawing_load_duration = Instant::now().saturating_duration_since(drawing_load_started);
    eprintln!("Drawing took {drawing_load_duration:?} to load and translate.");

    let picking_index = EntityIndex::new(&drawing);

    {
        eprintln!(
            "Loaded {} unique entities, {} total stroked shapes.",
            drawing.item_entity_map.len(),
            drawing
                .item_entity_map
                .iter()
                .flat_map(|(k, _v)| match drawing.graphics.get(*k) {
                    Some(GraphicsItem::FatShape(FatShape { subshapes, .. })) => subshapes.iter(),
                    _ => [].iter(),
                })
                .count()
        );
    }

    let mut app = SimpleVelloApp {
        context: RenderContext::new(),
        renderers: vec![],
        state: RenderState::Suspended(None),
        scene: Scene::new(),
        drawing,
        picking_index,
        pick: None,
        tv_environment: Default::default(),
        view_transform: Default::default(),
        defer_reprojection: Default::default(),
        view_scale: 1.0,
        gestures: Default::default(),
    };

    let event_loop = EventLoop::new()?;
    event_loop
        .run_app(&mut app)
        .expect("Couldn't run event loop");
    Ok(())
}

/// Helper function that creates a Winit window and returns it (wrapped in an Arc for sharing between threads)
fn create_winit_window(event_loop: &ActiveEventLoop) -> Arc<Window> {
    let attr = Window::default_attributes()
        .with_inner_size(LogicalSize::new(1044, 800))
        .with_resizable(true)
        .with_title("Vello Shapes");
    Arc::new(event_loop.create_window(attr).unwrap())
}

/// Helper function that creates a vello `Renderer` for a given `RenderContext` and `RenderSurface`
fn create_vello_renderer(render_cx: &RenderContext, surface: &RenderSurface<'_>) -> Renderer {
    Renderer::new(
        &render_cx.devices[surface.dev_id].device,
        RendererOptions {
            surface_format: Some(surface.format),
            use_cpu: false,
            antialiasing_support: vello::AaSupport::area_only(),
            num_init_threads: NonZeroUsize::new(1),
        },
    )
    .expect("Couldn't create renderer")
}

/// Update the transform/scale in all the items in a `GraphicsBag`.
fn update_transform(graphics: &mut GraphicsBag, transform: Affine, scale: f64) {
    // Update root transform.
    graphics.update_transform(Default::default(), transform);

    // Update default stroke.
    graphics.update_paint(
        Default::default(),
        FatPaint {
            // Unfortunately, post-transform stroke widths are not supported.
            stroke: Stroke::new(1.0 / scale),
            stroke_paint: Some(Color::BLACK.into()),
            fill_paint: None,
        },
    );
}

/// Bounding box index for entities.
struct EntityIndex {
    bounds_index: StaticAABB2DIndex<f64>,
    lines: Arc<[AnyShape]>,
    entity_mapping: Vec<EntityHandle>,
}

impl EntityIndex {
    fn new(d: &TDDrawing) -> Self {
        let build_started = Instant::now();

        let mut lines: Vec<AnyShape> = vec![];
        let mut entity_mapping = vec![];
        for (k, v) in d.item_entity_map.iter() {
            let Some(GraphicsItem::FatShape(FatShape { subshapes, .. })) = d.graphics.get(*k)
            else {
                continue;
            };
            for s in subshapes.iter() {
                entity_mapping.push(*v);
                lines.push(s.clone());
            }
        }
        let lines: Arc<[AnyShape]> = Arc::from(lines.as_slice());

        let bounds_index = compute_bounds_index(lines.clone());

        let build_duration = Instant::now().saturating_duration_since(build_started);
        eprintln!("Bounds index took {build_duration:?} to build.");

        Self {
            bounds_index,
            lines,
            entity_mapping,
        }
    }

    /// Pick entity that is closest to dp.
    fn pick(&self, dp: Point, sp: f64) -> Option<EntityHandle> {
        self.bounds_index
            .query(dp.x - sp, dp.y - sp, dp.x + sp, dp.y + sp)
            .iter()
            .filter(|i| self.lines[**i].dist_sq(dp) < (sp * sp))
            .reduce(|a, b| {
                if self.lines[*b].dist_sq(dp) < self.lines[*a].dist_sq(dp) {
                    b
                } else {
                    a
                }
            })
            .map(|i| self.entity_mapping[*i])
    }

    /// Query which entities' geometry overlaps with the bounds.
    fn query(&self, left: f64, top: f64, right: f64, bottom: f64) -> BTreeSet<EntityHandle> {
        self.bounds_index
            .query(left, top, right, bottom)
            .iter()
            .map(|l| self.entity_mapping[*l])
            .collect()
    }
}

use static_aabb2d_index::{StaticAABB2DIndex, StaticAABB2DIndexBuilder};

/// Compute an index of bounding boxes for shapes.
fn compute_bounds_index(lines: Arc<[AnyShape]>) -> StaticAABB2DIndex<f64> {
    let mut builder = StaticAABB2DIndexBuilder::new(lines.len());
    for shape in lines.as_ref() {
        let bbox = shape.bounding_box();
        builder.add(bbox.min_x(), bbox.min_y(), bbox.max_x(), bbox.max_y());
    }
    builder.build().unwrap()
}
