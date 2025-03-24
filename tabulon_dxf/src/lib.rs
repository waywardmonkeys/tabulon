// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! DXF loader for Tabulon

use dxf::{entities::EntityType, Drawing, DxfResult};
use tabulon::{
    peniko::{
        kurbo::{Affine, Arc as KurboArc, BezPath, Circle, Line, PathEl, Point, Vec2},
        Color,
    },
    shape::{AnyShape, SmallVec},
    text::{AttachmentPoint, FatText},
    DirectIsometry,
};

use parley::StyleSet;

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
        EntityType::Polyline(ref pl) => {
            use dxf::entities::Vertex;
            // FIXME: Polyline variable width and arcs, and a variety of other things.
            //        In some cases vertices might actually be indices?
            let vertices: Vec<&Vertex> = pl.vertices().collect();
            if vertices.len() >= 2 && !pl.is_polyface_mesh() && !pl.is_3d_polygon_mesh() {
                let mut bp = BezPath::new();
                bp.push(PathEl::MoveTo(point_from_dxf_point(&vertices[0].location)));
                for v in vertices.iter().skip(1) {
                    bp.push(PathEl::LineTo(point_from_dxf_point(&v.location)));
                }
                if pl.is_closed() {
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

/// Tabulon data for the drawing.
#[derive(Debug)]
pub struct TDDrawing {
    /// Collection of shapes for drawing lines.
    pub lines: SmallVec<[AnyShape; 1]>,
    /// Collection of text descriptions (from TEXT/MTEXT entities).
    pub texts: SmallVec<[FatText; 1]>,
}

use parley::{FontStyle, FontWeight, FontWidth, GenericFamily, StyleProperty};

/// Check if the font size of a [`StyleSet`] is zero.
fn style_size_is_zero(s: &StyleSet<Option<Color>>) -> bool {
    s.inner()
        .get(&core::mem::discriminant(&StyleProperty::FontSize(0_f32)))
        .is_none_or(|x| matches!(x, StyleProperty::FontSize(0_f32)))
}

/// Load a DXF from a path, and convert the entities in its enabled layers to Tabulon [`AnyShape`]s.
#[cfg(feature = "std")]
pub fn load_file_default_layers(path: impl AsRef<Path>) -> DxfResult<TDDrawing> {
    let mut lines = SmallVec::<[AnyShape; 1]>::new();
    let mut texts = SmallVec::<[FatText; 1]>::new();

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

    let styles: BTreeMap<&str, StyleSet<Option<Color>>> = drawing
        .styles()
        .map(
            #[allow(clippy::cast_possible_truncation, reason = "It doesn't matter")]
            |s| {
                // FIXME: I'm told this is actually the cap height and not the em size,
                //        at least for shx line fonts.
                // When this is zero, the height from the TEXT/MTEXT entity is used;
                // when this is nonzero, the height from the TXT/MTEXT is ignored.
                let size = s.text_height;
                let mut pstyle: StyleSet<Option<Color>> = StyleSet::new(size as f32);
                pstyle.insert(StyleProperty::FontWidth(FontWidth::from_ratio(
                    s.width_factor as f32,
                )));
                if s.oblique_angle != 0.0 {
                    pstyle.insert(StyleProperty::FontStyle(FontStyle::Oblique(Some(
                        s.oblique_angle as f32,
                    ))));
                }

                // TODO: Handle text_generation_flags somehow; My understanding is:
                //        - The second bit means the text is mirrored lengthwise
                //        - The third bit means the text is mirrored vertically

                // This is a selection of shx file names I've seen in the wild.
                //
                // TODO: We should probably eventually map to more correct fonts, or
                //       somehow match the outer metrics of these fonts more closely.
                //
                //       Sometimes the file names have the .shx, sometimes they do not,
                //       there appears to be neither rhyme nor reason to it.
                match s.primary_font_file_name.as_str() {
                    // Monospace version of txt.shx
                    "monotxt" | "monotxt.shx" => pstyle.insert(GenericFamily::Monospace.into()),
                    // Italic roman type lined once.
                    "italic" | "italic.shx" => {
                        pstyle.insert(GenericFamily::Serif.into());
                        pstyle.insert(StyleProperty::FontStyle(FontStyle::Italic))
                    }
                    // Roman (serif) type lined once.
                    "romans" | "romans.shx" => pstyle.insert(GenericFamily::Serif.into()),
                    // Condensed Roman type lined once.
                    "romanc" | "romanc.shx" => {
                        pstyle.insert(GenericFamily::Serif.into());
                        pstyle.insert(StyleProperty::FontWidth(FontWidth::CONDENSED))
                    }
                    // Roman type lined twice, seems like bold.
                    "romand" | "romand.shx" => {
                        pstyle.insert(GenericFamily::Serif.into());
                        pstyle.insert(StyleProperty::FontWeight(FontWeight::BOLD))
                    }
                    // Roman type lined thrice, seems like bolder.
                    "romant" | "romant.shx" => {
                        pstyle.insert(GenericFamily::Serif.into());
                        pstyle.insert(StyleProperty::FontWeight(FontWeight::EXTRA_BOLD))
                    }
                    "script" | "script.shx" => pstyle.insert(GenericFamily::Cursive.into()),
                    // Covers common "txt" | "txt.shx" | "simplex.shx" | "isocp.shx" | "gothic.shx"
                    _ => pstyle.insert(GenericFamily::SansSerif.into()),
                };

                (s.name.as_str(), pstyle)
            },
        )
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
            #[allow(clippy::cast_possible_truncation, reason = "It doesn't matter")]
            EntityType::MText(ref mt) => {
                // TODO: Parse MTEXT encoded characters to Unicode equivalents.
                // TODO: Set up background fills.
                // TODO: Handle inline style changes?
                // TODO: Handle columns.
                // TODO: Handle paragraph styles.
                // TODO: Handle rotation.
                let mut nt = mt.text.clone();
                for ext in mt.extended_text.iter() {
                    nt.push_str(ext);
                }
                let nt = nt
                    .replace("\\P", "\n")
                    .replace("\\A1;", "")
                    .replace("\\A0;", "");

                texts.push(FatText {
                    transform: Default::default(),
                    text: nt.into(),
                    // TODO: Map more styling information from the MText
                    style: styles.get(mt.text_style_name.as_str()).map_or_else(
                        || StyleSet::new(mt.initial_text_height as f32),
                        |s| {
                            if style_size_is_zero(s) {
                                let mut news = s.clone();
                                news.insert(StyleProperty::FontSize(mt.initial_text_height as f32));
                                news
                            } else {
                                s.clone()
                            }
                        },
                    ),
                    alignment: Default::default(),
                    insertion: DirectIsometry::new(
                        mt.rotation_angle * (core::f64::consts::PI / 180.),
                        point_from_dxf_point(&mt.insertion_point).to_vec2(),
                    ),
                    max_inline_size: (mt.reference_rectangle_width != 0.0)
                        .then_some(mt.reference_rectangle_width as f32),
                    attachment_point: dxf_attachment_point_to_tabulon(mt.attachment_point),
                });
            }
            EntityType::Text(ref t) => {
                // TODO: Handle second_alignment_point etc?
                // TODO: Handle relative_x_scale_factor.
                #[allow(clippy::cast_possible_truncation, reason = "It doesn't matter")]
                texts.push(FatText {
                    transform: Default::default(),
                    text: t.value.clone().into(),
                    style: styles.get(t.text_style_name.as_str()).map_or_else(
                        || StyleSet::new(t.text_height as f32),
                        |s| {
                            let mut sized = if style_size_is_zero(s) {
                                let mut news = s.clone();
                                news.insert(StyleProperty::FontSize(t.text_height as f32));
                                news
                            } else {
                                s.clone()
                            };
                            if t.oblique_angle != 0.0 {
                                sized.insert(StyleProperty::FontStyle(FontStyle::Oblique(Some(
                                    t.oblique_angle as f32,
                                ))));
                            }
                            sized
                        },
                    ),
                    alignment: Default::default(),
                    insertion: DirectIsometry::new(
                        t.rotation * (core::f64::consts::PI / 180.),
                        point_from_dxf_point(&t.location).to_vec2(),
                    ),
                    max_inline_size: None,
                    attachment_point: Default::default(),
                });
            }
            _ => {
                if let Some(s) = shape_from_entity(e) {
                    lines.push(s);
                }
            }
        }
    }

    Ok(TDDrawing { lines, texts })
}

/// Convert a [`dxf::enums::AttachmentPoint`] to a [`tabulon::text::AttachmentPoint`].
fn dxf_attachment_point_to_tabulon(
    attachment_point: dxf::enums::AttachmentPoint,
) -> AttachmentPoint {
    use dxf::enums::AttachmentPoint as d;
    use AttachmentPoint::*;
    match attachment_point {
        d::TopLeft => TopLeft,
        d::TopCenter => TopCenter,
        d::TopRight => TopRight,
        d::MiddleLeft => MiddleLeft,
        d::MiddleCenter => MiddleCenter,
        d::MiddleRight => MiddleRight,
        d::BottomLeft => BottomLeft,
        d::BottomCenter => BottomCenter,
        d::BottomRight => BottomRight,
    }
}

#[cfg(test)]
mod tests {}
