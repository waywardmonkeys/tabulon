// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello rendering utilities for Tabulon.

use tabulon::{
    graphics_bag::{GraphicsBag, GraphicsItem},
    render_layer::RenderLayer,
    shape::{AnyShape, FatPaint, FatShape},
};

use vello::Scene;

/// Add a [`RenderLayer`] to a Vello [`Scene`].
pub fn add_render_layer_to_scene(
    scene: &mut Scene,
    graphics: &GraphicsBag,
    render_layer: &RenderLayer,
) {
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
            }
        }
    }
}
