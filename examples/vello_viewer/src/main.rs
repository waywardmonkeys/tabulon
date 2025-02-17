// Copyright 2024 the Vello Authors
// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! DXF viewer

use anyhow::Result;
use std::num::NonZeroUsize;
use std::sync::Arc;
use vello::kurbo::{Affine, Arc as KurboArc, BezPath, Circle, Line, PathEl, Point, Stroke, Vec2};
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

    /// Collection of shapes to be stroked with the default line style.
    lines: SmallVec<[AnyShape; 1]>,

    /// View transform of the drawing.
    view_transform: Affine,
    /// View scale of the drawing.
    view_scale: f64,
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
        let surface_future = self.context.create_surface(
            window.clone(),
            size.width,
            size.height,
            wgpu::PresentMode::AutoVsync,
        );
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

        self.lines = {
            use dxf::{entities::EntityType, Drawing};

            let drawing = Drawing::load_file(
                std::env::args()
                    .next_back()
                    .expect("Please provide a path in the last argument."),
            )
            .unwrap();

            let mut lines = SmallVec::<[AnyShape; 1]>::new();

            for e in drawing.entities() {
                match e.specific {
                    EntityType::Arc(ref a) => {
                        let dxf::entities::Arc {
                            center,
                            radius,
                            start_angle,
                            end_angle,
                            ..
                        } = a.clone();
                        lines.push(
                            KurboArc {
                                center: Point {
                                    x: center.x,
                                    y: center.y,
                                },
                                radii: Vec2::new(radius, radius),
                                start_angle,
                                // FIXME: don't know if this is correct.
                                sweep_angle: end_angle,
                                x_rotation: 0.0,
                            }
                            .into(),
                        );
                    }
                    EntityType::Line(ref line) => {
                        let p0 = {
                            let dxf::Point { x, y, .. } = line.p1;
                            Point { x, y }
                        };
                        let p1 = {
                            let dxf::Point { x, y, .. } = line.p2;
                            Point { x, y }
                        };
                        let l = Line { p0, p1 };
                        lines.push(l.into());
                    }
                    EntityType::Circle(ref circle) => {
                        let center = {
                            let dxf::Point { x, y, .. } = circle.center;
                            Point { x, y }
                        };
                        let c = Circle {
                            center,
                            radius: circle.radius,
                        };
                        lines.push(c.into());
                    }
                    EntityType::LwPolyline(ref lwp) => {
                        // FIXME: LwPolyline supports variable width and arcs.
                        if lwp.vertices.len() >= 2 {
                            let mut bp = BezPath::new();
                            fn lwp_vertex_to_point(
                                dxf::LwPolylineVertex { x, y, .. }: dxf::LwPolylineVertex,
                            ) -> Point {
                                Point { x, y }
                            }
                            bp.push(PathEl::MoveTo(lwp_vertex_to_point(lwp.vertices[0])));
                            for i in 1..(lwp.vertices.len() - 1) {
                                bp.push(PathEl::LineTo(lwp_vertex_to_point(lwp.vertices[i])));
                            }
                            lines.push(bp.into());
                        }
                    }
                    _ => {
                        eprintln!("unhandled entity {:?}", e.specific);
                    }
                }
            }

            lines
        };

        let bounds = self
            .lines
            .iter()
            .map(AnyShape::bounding_box)
            .fold(vello::kurbo::Rect::default(), |a, x| a.union(x));

        self.view_scale = (size.height as f64 / bounds.size().height)
            .min(size.width as f64 / bounds.size().width);

        self.view_transform = Affine::translate(Vec2 {
            x: -bounds.min_x(),
            y: -bounds.min_y(),
        })
        .then_scale(self.view_scale);

        self.scene.reset();
        add_shapes_to_scene(
            &mut self.scene,
            self.view_transform,
            &self.lines,
            1. / self.view_scale,
        );
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
        let surface = match &mut self.state {
            RenderState::Active { surface, window } if window.id() == window_id => surface,
            _ => return,
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                self.context
                    .resize_surface(surface, size.width, size.height);

                let bounds = self
                    .lines
                    .iter()
                    .map(AnyShape::bounding_box)
                    .fold(vello::kurbo::Rect::default(), |a, x| a.union(x));

                self.view_scale = (size.height as f64 / bounds.size().height)
                    .min(size.width as f64 / bounds.size().width);

                self.view_transform = Affine::translate(Vec2 {
                    x: -bounds.min_x(),
                    y: -bounds.min_y(),
                })
                .then_scale(self.view_scale);

                self.scene.reset();
                add_shapes_to_scene(
                    &mut self.scene,
                    self.view_transform,
                    &self.lines,
                    1. / self.view_scale,
                );
            }

            WindowEvent::CursorMoved { position, .. } => {
                let p = {
                    let winit::dpi::PhysicalPosition::<f64> { x, y } = position;
                    Point { x, y }
                };

                let mut gb = GraphicsBag::default();
                gb.push(FatShape {
                    transform: self.view_transform,
                    paint: FatPaint {
                        stroke: Stroke::new(1.0 / self.view_scale),
                        stroke_paint: Some(Color::WHITE.into()),
                        fill_paint: None,
                    },
                    subshapes: self.lines.clone(),
                });

                if let Some(item) = gb.get(0) {
                    match item {
                        GraphicsItem::FatShape(s) => {
                            if let Some(p) = s.pick(p, 10000.).first() {
                                println!("closest item: {:?}", s.subshapes[*p]);
                            }
                        }
                    }
                }
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
                            base_color: palette::css::BLACK, // Background color
                            width,
                            height,
                            antialiasing_method: AaConfig::Msaa16,
                        },
                    )
                    .expect("failed to render to surface");

                surface_texture.present();

                device_handle.device.poll(wgpu::Maintain::Poll);
            }
            _ => {}
        }
    }
}

fn main() -> Result<()> {
    let mut app = SimpleVelloApp {
        context: RenderContext::new(),
        renderers: vec![],
        state: RenderState::Suspended(None),
        scene: Scene::new(),
        lines: Default::default(),
        view_transform: Default::default(),
        view_scale: 1.0,
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
            antialiasing_support: vello::AaSupport::all(),
            num_init_threads: NonZeroUsize::new(1),
        },
    )
    .expect("Couldn't create renderer")
}

use tabulon::{
    graphics_bag::{GraphicsBag, GraphicsItem},
    render_layer::RenderLayer,
    shape::{AnyShape, FatPaint, FatShape, SmallVec},
};

/// Add shapes to a vello scene. This does not actually render the shapes, but adds them
/// to the Scene data structure which represents a set of objects to draw.
fn add_shapes_to_scene(
    scene: &mut Scene,
    transform: Affine,
    lines: &SmallVec<[AnyShape; 1]>,
    default_line_weight: f64,
) {
    let mut rl = RenderLayer::default();
    let mut gb = GraphicsBag::default();

    // Draw some lines
    rl.push_with_bag(
        &mut gb,
        FatShape {
            transform,
            paint: FatPaint {
                stroke: Stroke::new(default_line_weight),
                stroke_paint: Some(Color::WHITE.into()),
                fill_paint: None,
            },
            subshapes: lines.clone(), // FIXME: very bad
        },
    );

    // AnyShape is an enum and there's no elegant way to reverse this to an impl Shape.
    macro_rules! render_anyshape_match {
        ( $paint:ident, $transform:ident, $subshape:ident, $($name:ident)|* ) => {
            let FatPaint {
                stroke,
                stroke_paint,
                fill_paint,
            } = $paint;

            match $subshape {
                $(AnyShape::$name(x) =>  {
                    if let Some(stroke_paint) = stroke_paint {
                        scene.stroke(&stroke, *$transform, stroke_paint, None, &x);
                    }
                    if let Some(fill_paint) = fill_paint {
                        scene.fill(
                            vello::peniko::Fill::NonZero,
                            *$transform,
                            fill_paint,
                            None,
                            &x,
                        );
                    }
                }),*
            }
        }
    }

    for idx in rl.indices {
        if let Some(ref gi) = gb.get(idx) {
            match gi {
                GraphicsItem::FatShape(FatShape {
                    paint,
                    transform,
                    subshapes,
                }) => {
                    for subshape in subshapes {
                        render_anyshape_match!(
                            paint,
                            transform,
                            subshape,
                            Arc | BezPath
                                | Circle
                                | CircleSegment
                                | CubicBez
                                | Ellipse
                                | Line
                                | PathSeg
                                | QuadBez
                                | Rect
                                | RoundedRect
                        );
                    }
                }
            }
        }
    }
}
