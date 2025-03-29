// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello rendering utilities for Tabulon.

use tabulon::{
    graphics_bag::{GraphicsBag, GraphicsItem},
    peniko::{
        kurbo::{Affine, Size},
        Color, Fill,
    },
    render_layer::RenderLayer,
    shape::{AnyShape, FatPaint, FatShape},
    text::FatText,
};

use parley::{FontContext, LayoutContext, PositionedLayoutItem};
use vello::{peniko::Fill::NonZero, Scene};

/// Expensive state for rendering.
#[derive(Default)]
#[allow(
    missing_debug_implementations,
    reason = "Not useful, and members don't implement Debug."
)]
pub struct Environment {
    /// Font context.
    ///
    /// This contains a font collection that is expensive to reproduce.
    pub(crate) font_cx: FontContext,
    /// Layout context.
    pub(crate) layout_cx: LayoutContext<Option<Color>>,
}

impl Environment {
    /// Add a [`RenderLayer`] to a Vello [`Scene`].
    pub fn add_render_layer_to_scene(
        &mut self,
        scene: &mut Scene,
        graphics: &GraphicsBag,
        render_layer: &RenderLayer,
    ) {
        let Self { font_cx, layout_cx } = self;

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
                        if let Some(fill_paint) = fill_paint {
                            scene.fill(NonZero, *$transform, fill_paint, None, &x);
                        }
                        if let Some(stroke_paint) = stroke_paint {
                            scene.stroke(&stroke, *$transform, stroke_paint, None, &x);
                        }
                    }),*
                }
            }
        }

        for idx in &render_layer.indices {
            if let Some(ref gi) = graphics.get(*idx) {
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
                    GraphicsItem::FatText(FatText {
                        transform,
                        text,
                        style,
                        max_inline_size,
                        alignment,
                        insertion,
                        attachment_point,
                    }) => {
                        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0);
                        for prop in style.inner().values() {
                            builder.push_default(prop.to_owned());
                        }
                        let mut layout = builder.build(text);
                        layout.break_all_lines(*max_inline_size);
                        layout.align(*max_inline_size, *alignment, Default::default());
                        let layout_size = Size {
                            width: max_inline_size.unwrap_or(layout.width()) as f64,
                            height: layout.height() as f64,
                        };

                        let placement_transform =
                            Affine::translate(-attachment_point.select(layout_size))
                                * Affine::from(*insertion);

                        for line in layout.lines() {
                            for item in line.items() {
                                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                                    continue;
                                };

                                let mut x = glyph_run.offset();
                                let y = glyph_run.baseline();
                                let run = glyph_run.run();
                                let synthesis = run.synthesis();
                                scene
                                    .draw_glyphs(run.font())
                                    // TODO: Color will come from styled text.
                                    .brush(Color::WHITE)
                                    .hint(false)
                                    .transform(*transform * placement_transform)
                                    .glyph_transform(synthesis.skew().map(|angle| {
                                        Affine::skew(angle.to_radians().tan() as f64, 0.0)
                                    }))
                                    .font_size(run.font_size())
                                    .normalized_coords(run.normalized_coords())
                                    .draw(
                                        Fill::NonZero,
                                        glyph_run.glyphs().map(|g| {
                                            let gx = x + g.x;
                                            let gy = y - g.y;
                                            x += g.advance;
                                            vello::Glyph {
                                                id: g.id as _,
                                                x: gx,
                                                y: gy,
                                            }
                                        }),
                                    );
                            }
                        }
                    }
                }
            }
        }
    }
}
