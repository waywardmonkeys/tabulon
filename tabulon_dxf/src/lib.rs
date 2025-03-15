// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! DXF loader for Tabulon

use dxf::{entities::EntityType, Drawing, DxfResult};
use tabulon::{
    peniko::kurbo::{Affine, Arc as KurboArc, BezPath, Circle, Line, PathEl, Point, Vec2},
    shape::{AnyShape, SmallVec},
};

extern crate alloc;
use alloc::collections::{btree_map::BTreeMap, btree_set::BTreeSet};

#[cfg(feature = "std")]
use std::path::Path;

/// Convert entity to lines
pub fn shape_from_entity(e: &dxf::entities::Entity) -> Option<AnyShape> {
    match e.specific {
        EntityType::Arc(ref a) => {
            let dxf::entities::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ..
            } = a.clone();
            Some(
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
            )
        }
        EntityType::Line(ref line) => {
            let p0 = point_from_dxf_point(&line.p1);
            let p1 = point_from_dxf_point(&line.p2);
            Some(Line { p0, p1 }.into())
        }
        EntityType::Circle(ref circle) => {
            let center = point_from_dxf_point(&circle.center);
            let c = Circle {
                center,
                radius: circle.radius,
            };
            Some(c.into())
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
                for v in lwp.vertices.iter().skip(1) {
                    bp.push(PathEl::LineTo(lwp_vertex_to_point(*v)));
                }
                if lwp.is_closed() {
                    bp.close_path();
                }
                Some(bp.into())
            } else {
                None
            }
        }
        _ => {
            // eprintln!(
            //     "unhandled entity {} {} {:?}",
            //     e.common.handle.0, e.common.layer, e.specific
            // );
            None
        }
    }
}

/// Make a [`Point`] from the x and y of a [`dxf::Point`].
pub fn point_from_dxf_point(p: &dxf::Point) -> Point {
    let dxf::Point { x, y, .. } = *p;
    Point { x, y }
}

/// Load a DXF from a path, and convert the entities in its enabled layers to Tabulon [`AnyShape`]s.
#[cfg(feature = "std")]
pub fn load_file_default_layers(path: impl AsRef<Path>) -> DxfResult<SmallVec<[AnyShape; 1]>> {
    let mut lines = SmallVec::<[AnyShape; 1]>::new();

    let drawing = Drawing::load_file(path)?;

    let visible_layers: BTreeSet<&str> = drawing
        .layers()
        .filter_map(|l| l.is_layer_on.then_some(l.name.as_str()))
        .collect();

    // FIXME: It's conceivable that a BLOCK may have an INSERT so
    //        we should figure out something sane to do with that.
    let blocks: BTreeMap<&str, SmallVec<[AnyShape; 4]>> = drawing
        .blocks()
        .map(|b| {
            (
                b.name.as_str(),
                b.entities.iter().filter_map(shape_from_entity).collect(),
            )
        })
        .collect();

    for e in drawing.entities() {
        if !(e.common.layer.is_empty() || visible_layers.contains(e.common.layer.as_str())) {
            continue;
        }
        match e.specific {
            EntityType::Insert(ref ins) => {
                if let Some(b) = blocks.get(ins.name.as_str()) {
                    let base_transform =
                        Affine::scale_non_uniform(ins.x_scale_factor, ins.y_scale_factor);
                    let location = point_from_dxf_point(&ins.location);
                    for i in 0..ins.row_count {
                        for j in 0..ins.column_count {
                            let transform = base_transform
                                .then_translate(Vec2::new(
                                    j as f64 * ins.column_spacing,
                                    i as f64 * ins.row_spacing,
                                ))
                                .then_rotate(ins.rotation * (core::f64::consts::PI / 180.))
                                .then_translate(location.to_vec2());
                            for s in b {
                                lines.push(s.transform(transform));
                            }
                        }
                    }
                }
            }
            _ => {
                if let Some(s) = shape_from_entity(e) {
                    lines.push(s);
                }
            }
        }
    }

    Ok(lines)
}

#[cfg(test)]
mod tests {}
