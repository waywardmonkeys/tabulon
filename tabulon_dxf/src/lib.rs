// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! DXF loader for Tabulon

use dxf::{entities::EntityType, Drawing, DxfResult};

use tabulon::{
    peniko::{
        kurbo::{Affine, Arc, BezPath, Circle, Line, PathEl, Point, Vec2, DEFAULT_ACCURACY},
        Color,
    },
    render_layer::RenderLayer,
    shape::{AnyShape, FatPaint, FatShape},
    text::{AttachmentPoint, FatText},
    DirectIsometry, GraphicsBag, GraphicsItem, ItemHandle, PaintHandle,
};

use joto_constants::u64::MICROMETER;
use parley::{Alignment, StyleSet};

extern crate alloc;
use alloc::{
    collections::{btree_map::BTreeMap, btree_set::BTreeSet},
    sync,
};

#[cfg(feature = "std")]
use std::path::Path;

use core::num::NonZeroU64;

mod aci_palette;
use aci_palette::ACI;

/// A valid handle for an [`Entity`](dxf::entities::Entity) present in the drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntityHandle(pub(crate) NonZeroU64);

/// A valid handle for a [`Layer`](dxf::tables::Layer) present in the drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LayerHandle(pub(crate) NonZeroU64);

/// Convert entity to lines
pub fn shape_from_entity(e: &dxf::entities::Entity) -> Option<AnyShape> {
    match e.specific {
        EntityType::Arc(ref a) => {
            // FIXME: currently only support viewing from +Z.
            if a.normal.z != 1.0 {
                return None;
            }

            let dxf::entities::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ..
            } = a.clone();
            Some(
                Arc {
                    center: point_from_dxf_point(&center),
                    radii: Vec2 {
                        x: radius,
                        y: radius,
                    },
                    // DXF is y-up, so these are originally counterclockwise.
                    start_angle: -start_angle.to_radians(),
                    sweep_angle: -(end_angle - start_angle).rem_euclid(360.0).to_radians(),
                    x_rotation: 0.0,
                }
                .into(),
            )
        }
        EntityType::Line(ref line) => {
            // FIXME: currently only support viewing from +Z.
            if line.extrusion_direction.z != 1.0 {
                return None;
            }

            let p0 = point_from_dxf_point(&line.p1);
            let p1 = point_from_dxf_point(&line.p2);
            Some(Line { p0, p1 }.into())
        }
        EntityType::Circle(ref circle) => {
            // FIXME: currently only support viewing from +Z.
            if circle.normal.z != 1.0 {
                return None;
            }

            let center = point_from_dxf_point(&circle.center);
            let c = Circle {
                center,
                radius: circle.radius,
            };
            Some(c.into())
        }
        EntityType::LwPolyline(ref lwp) => {
            // FIXME: currently only support viewing from +Z.
            if lwp.extrusion_direction.z != 1.0 {
                return None;
            }

            fn lwp_vertex_to_point(
                dxf::LwPolylineVertex { x, y, .. }: dxf::LwPolylineVertex,
            ) -> Point {
                Point { x, y: -y }
            }

            if lwp.vertices.len() < 2 {
                return None;
            }

            let mut bp = BezPath::new();
            bp.push(PathEl::MoveTo(lwp_vertex_to_point(lwp.vertices[0])));

            for w in lwp.vertices.windows(2) {
                let current = &w[0];
                let next = &w[1];
                let start = lwp_vertex_to_point(*current);
                let end = lwp_vertex_to_point(*next);

                // Bulge needs reversed because DXF is y-up
                let bulge = -current.bulge;
                add_poly_segment(&mut bp, start, end, bulge);
            }

            if lwp.is_closed() {
                bp.close_path();
            }

            Some(bp.into())
        }
        EntityType::Polyline(ref pl) => {
            // FIXME: currently only support viewing from +Z.
            if pl.normal.z != 1.0 {
                return None;
            }

            use dxf::entities::Vertex;
            // FIXME: Polyline variable width and arcs, and a variety of other things.
            //        In some cases vertices might actually be indices?
            if pl.is_polyface_mesh() || pl.is_3d_polygon_mesh() {
                return None;
            }

            let vertices: Vec<&Vertex> = pl.vertices().collect();
            if vertices.len() < 2 {
                return None;
            }

            let mut bp = BezPath::new();
            bp.push(PathEl::MoveTo(point_from_dxf_point(&vertices[0].location)));

            for w in vertices.windows(2) {
                let current = &w[0];
                let next = &w[1];
                let start = point_from_dxf_point(&current.location);
                let end = point_from_dxf_point(&next.location);

                // Bulge needs reversed because DXF is y-up
                let bulge = -current.bulge;
                add_poly_segment(&mut bp, start, end, bulge);
            }

            if pl.is_closed() {
                bp.close_path();
            }

            Some(bp.into())
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

/// Add a polyline segment to a `BezPath`, taking bulge into account.
fn add_poly_segment(bp: &mut BezPath, start: Point, end: Point, bulge: f64) {
    if bulge == 0.0 {
        bp.push(PathEl::LineTo(end));
        return;
    }

    let theta = 4.0 * bulge.atan();
    if theta.abs() < 1e-6 {
        bp.push(PathEl::LineTo(end));
        return;
    }

    let v = end - start;
    let d = v.hypot();
    if d < 1e-10 {
        // Points are too dang close.
        bp.push(PathEl::LineTo(end));
        return;
    }

    let r = d / (2.0 * (theta / 2.0).sin().abs());

    let center = {
        let s = bulge.signum();
        let perp = Vec2 {
            x: -s * v.y,
            y: s * v.x,
        };
        let h = r * (theta / 2.0).cos();
        let midpoint = (start.to_vec2() + end.to_vec2()) / 2.0;
        (midpoint + (h / d) * perp).to_point()
    };

    let start_angle = (start - center.to_vec2()).to_vec2().atan2();

    let arc = Arc {
        center,
        radii: Vec2 { x: r, y: r },
        start_angle,
        sweep_angle: theta,
        x_rotation: 0.0,
    };

    arc.to_cubic_beziers(DEFAULT_ACCURACY, |p1, p2, p3| {
        bp.curve_to(p1, p2, p3);
    });
}

/// Make a [`Point`] from the x and y of a [`dxf::Point`].
pub fn point_from_dxf_point(p: &dxf::Point) -> Point {
    let dxf::Point { x, y, .. } = *p;
    Point { x, y: -y }
}

/// Provide information about a drawing after loading it.
#[allow(
    missing_debug_implementations,
    reason = "Not particularly useful, and members don't implement Debug."
)]
pub struct DrawingInfo {
    drawing: Drawing,
}

impl DrawingInfo {
    pub(crate) fn new(drawing: Drawing) -> Self {
        Self { drawing }
    }

    /// Get an entity in the drawing.
    pub fn get_entity(&self, eh: EntityHandle) -> &dxf::entities::Entity {
        let dxf::DrawingItem::Entity(e) = self
            .drawing
            .item_by_handle(dxf::Handle(eh.0.get()))
            .unwrap()
        else {
            unreachable!();
        };
        e
    }
}

/// Tabulon data for the drawing.
#[allow(
    missing_debug_implementations,
    reason = "Not particularly useful, and members don't implement Debug."
)]
pub struct TDDrawing {
    /// `GraphicsBag` containing drawn items.
    pub graphics: GraphicsBag,
    /// Mapping from graphics items to entity handles.
    pub item_entity_map: BTreeMap<ItemHandle, EntityHandle>,
    /// Entities for layers.
    pub entity_layer_map: BTreeMap<EntityHandle, LayerHandle>,
    /// Render layer in drawing order.
    pub render_layer: RenderLayer,
    /// Enabled layers.
    pub enabled_layers: BTreeSet<LayerHandle>,
    /// Layer names.
    pub layer_names: BTreeMap<LayerHandle, sync::Arc<str>>,
    /// Drawing information object.
    pub info: DrawingInfo,
    /// Paints that need stroke widths computed relative to view.
    ///
    /// Format is pairs of physical stroke widths with the target `PaintHandle`.
    /// Physical widths are expressed in (iota)[`joto_constants::u64::IOTA`].
    pub restroke_paints: sync::Arc<[(u64, PaintHandle)]>,
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
#[tracing::instrument(skip_all)]
pub fn load_file_default_layers(path: impl AsRef<Path>) -> DxfResult<TDDrawing> {
    let mut gb = GraphicsBag::default();
    let mut rl = RenderLayer::default();
    let mut item_entity_map = BTreeMap::new();
    let mut entity_layer_map = BTreeMap::new();

    // FIXME: use real colors and line widths, and expose information for line scaling.
    //        This currently sets the paint at position 0/default in the palette.
    let _paint = gb.register_paint(FatPaint {
        stroke: Default::default(),
        stroke_paint: Some(Color::BLACK.into()),
        fill_paint: None,
    });

    let drawing = Drawing::load_file(path)?;

    let visible_layers: BTreeSet<&str> = drawing
        .layers()
        .filter_map(|l| l.is_layer_on.then_some(l.name.as_str()))
        .collect();

    let enabled_layers = drawing
        .layers()
        .filter_map(|l| {
            l.is_layer_on
                .then_some(LayerHandle(NonZeroU64::new(l.handle.0).unwrap()))
        })
        .collect();

    let layer_names = drawing
        .layers()
        .map(|l| {
            (
                LayerHandle(NonZeroU64::new(l.handle.0).unwrap()),
                l.name.as_str().into(),
            )
        })
        .collect();

    let handle_for_layer_name: BTreeMap<&str, LayerHandle> = drawing
        .layers()
        .map(|l| {
            (
                l.name.as_str(),
                LayerHandle(NonZeroU64::new(l.handle.0).unwrap()),
            )
        })
        .collect();

    // FIXME: It's conceivable that a BLOCK may have an INSERT so
    //        we should figure out something sane to do with that.
    let blocks: BTreeMap<&str, Vec<AnyShape>> = drawing
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

    // Paints keyed on concrete rgba color, and concrete line width (in iotas).
    let mut paints: BTreeMap<(u32, u64), PaintHandle> = BTreeMap::new();

    for e in drawing.entities() {
        if !e.common.is_visible
            || !(e.common.layer.is_empty() || visible_layers.contains(e.common.layer.as_str()))
        {
            continue;
        }

        let eh = EntityHandle(NonZeroU64::new(e.common.handle.0).unwrap());
        let lh = handle_for_layer_name[e.common.layer.as_str()];

        let Some(dxf::DrawingItem::Layer(layer)) = drawing.item_by_handle(dxf::Handle(lh.0.get()))
        else {
            // LayerHandle is known valid at this point.
            unreachable!();
        };

        // Get or create the appropriate PaintHandle for this entity.
        let entity_paint = {
            // Resolve color.

            let opaque_color = if e.common.color.is_by_entity() {
                e.common.color_24_bit as u32
            } else if e.common.color.is_by_layer() {
                if let Some(i) = layer.color.index() {
                    ACI[i as usize]
                } else {
                    u32::MAX
                }
            } else {
                u32::MAX
            };
            let combined_color =
                (opaque_color << 8) | (0xFF - (e.common.transparency as u32 & 0xFF));

            // Resolve line width.

            const LWDEFAULT: u64 = 250 * MICROMETER;

            let lwconcrete = match e.common.lineweight_enum_value {
                -3 => LWDEFAULT,
                // BYLAYER.
                -2 => {
                    if layer.line_weight.raw_value() <= 0 {
                        // BYLAYER and BYBLOCK are both meaningless in a layer,
                        // therefore, use the default for all enumerations.
                        LWDEFAULT
                    } else {
                        layer.line_weight.raw_value() as u64 * 10 * MICROMETER
                    }
                }
                // BYBLOCK Should not occur at the entity level, use default.
                -1 => LWDEFAULT,
                // Invalid enumerations, use default.
                i if i <= 0 => LWDEFAULT,
                i => i as u64 * 10 * MICROMETER,
            };

            let r = ((combined_color >> 24) & 0xFF) as u8;
            let g = ((combined_color >> 16) & 0xFF) as u8;
            let b = ((combined_color >> 8) & 0xFF) as u8;
            let a = (combined_color & 0xFF) as u8;

            *paints
                .entry((combined_color, lwconcrete))
                .or_insert_with(|| {
                    // At first these do not have stroke width, this needs to be set afterward.
                    gb.register_paint(FatPaint {
                        stroke_paint: Some(Color::from_rgba8(r, g, b, a).into()),
                        ..Default::default()
                    })
                })
        };

        let mut push_item = |item: GraphicsItem| {
            let ih = rl.push_with_bag(&mut gb, item);
            item_entity_map.insert(ih, eh);
            entity_layer_map.insert(eh, lh);
        };

        match e.specific {
            EntityType::Insert(ref ins) => {
                // FIXME: currently only support viewing from +Z.
                if ins.extrusion_direction.z != 1.0 {
                    continue;
                }

                if let Some(b) = blocks.get(ins.name.as_str()) {
                    let mut lines = BezPath::new();
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
                                .then_rotate(-ins.rotation.to_radians())
                                .then_translate(location.to_vec2());
                            for s in b {
                                lines.extend(s.transform(transform).to_path().iter());
                            }
                        }
                    }

                    push_item(
                        FatShape {
                            subshapes: sync::Arc::from([lines.into()]),
                            paint: entity_paint,
                            ..Default::default()
                        }
                        .into(),
                    );
                }
            }
            #[allow(clippy::cast_possible_truncation, reason = "It doesn't matter")]
            EntityType::MText(ref mt) => {
                // FIXME: currently only support viewing from +Z.
                if mt.extrusion_direction.z != 1.0 {
                    continue;
                }

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

                // TODO: Implement a shared parser for scanning formatting codes into styled text
                //       and doing unicode substitution for special character codes.
                let nt = nt
                    .replace("%%c", "∅")
                    .replace("%%d", "°")
                    .replace("%%p", "±")
                    .replace("%%C", "∅")
                    .replace("%%D", "°")
                    .replace("%%P", "±")
                    .replace("%%%", "%")
                    // TODO: Implement start/stop underline with styled text.
                    .replace("\\L", "")
                    .replace("\\l", "")
                    // TODO: Implement start/stop overline with styled text.
                    .replace("\\O", "")
                    .replace("\\o", "")
                    // TODO: Implement start/stop strikethrough with styled text.
                    .replace("\\S", "")
                    .replace("\\s", "")
                    .replace("\\P", "\n")
                    .replace("\\A1;", "")
                    .replace("\\A0;", "");

                let x_angle = Vec2 {
                    x: mt.x_axis_direction.x,
                    y: -mt.x_axis_direction.y,
                }
                .atan2();

                let attachment_point = dxf_attachment_point_to_tabulon(mt.attachment_point);

                // In DXF, the text alignment is also decided by the attachment point.
                let alignment = {
                    use Alignment::*;
                    use AttachmentPoint::*;
                    match attachment_point {
                        TopCenter | MiddleCenter | BottomCenter => Middle,
                        TopLeft | MiddleLeft | BottomLeft => Left,
                        TopRight | MiddleRight | BottomRight => Right,
                    }
                };

                let max_inline_size = if alignment == Alignment::Middle {
                    None
                } else {
                    match mt.column_type {
                        0 => (mt.reference_rectangle_width != 0.0)
                            .then_some(mt.reference_rectangle_width as f32),
                        1 => (mt.column_width != 0.0).then_some(mt.column_width as f32),
                        _ => None,
                    }
                };

                push_item(
                    FatText {
                        transform: Default::default(),
                        text: nt.into(),
                        // TODO: Map more styling information from the MText
                        style: styles.get(mt.text_style_name.as_str()).map_or_else(
                            || StyleSet::new(mt.initial_text_height as f32),
                            |s| {
                                if style_size_is_zero(s) {
                                    let mut news = s.clone();
                                    news.insert(StyleProperty::FontSize(
                                        mt.initial_text_height as f32,
                                    ));
                                    news
                                } else {
                                    s.clone()
                                }
                            },
                        ),
                        alignment,
                        insertion: DirectIsometry::new(
                            // As far as I'm aware, x_axis_direction and rotation are exclusive.
                            -mt.rotation_angle.to_radians() + x_angle,
                            point_from_dxf_point(&mt.insertion_point).to_vec2(),
                        ),
                        max_inline_size,
                        attachment_point,
                    }
                    .into(),
                );
            }
            EntityType::Text(ref t) => {
                // FIXME: currently only support viewing from +Z.
                if t.normal.z != 1.0 {
                    continue;
                }

                // TODO: Handle second_alignment_point etc?
                // TODO: Handle relative_x_scale_factor.

                // TODO: Implement a shared parser for scanning formatting codes into styled text
                //       and doing unicode substitution for special character codes.
                let text = t
                    .value
                    .replace("%%c", "∅")
                    .replace("%%d", "°")
                    .replace("%%p", "±")
                    .replace("%%C", "∅")
                    .replace("%%D", "°")
                    .replace("%%P", "±")
                    .replace("%%%", "%")
                    // TODO: implement toggle underline with styled text.
                    .replace("%%u", "")
                    // TODO: implement toggle overline with styled text.
                    .replace("%%o", "");

                #[allow(clippy::cast_possible_truncation, reason = "It doesn't matter")]
                push_item(
                    FatText {
                        transform: Default::default(),
                        text: text.into(),
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
                                    sized.insert(StyleProperty::FontStyle(FontStyle::Oblique(
                                        Some(t.oblique_angle as f32),
                                    )));
                                }
                                sized
                            },
                        ),
                        alignment: Default::default(),
                        insertion: DirectIsometry::new(
                            -t.rotation.to_radians(),
                            point_from_dxf_point(&t.location).to_vec2(),
                        ),
                        max_inline_size: None,
                        attachment_point: Default::default(),
                    }
                    .into(),
                );
            }
            _ => {
                if let Some(s) = shape_from_entity(e) {
                    push_item(
                        FatShape {
                            subshapes: sync::Arc::from([s]),
                            paint: entity_paint,
                            ..Default::default()
                        }
                        .into(),
                    );
                }
            }
        }
    }

    let restroke_paints: Vec<(u64, PaintHandle)> =
        paints.iter().map(|((_, w), h)| (*w, *h)).collect();

    Ok(TDDrawing {
        graphics: gb,
        render_layer: rl,
        item_entity_map,
        entity_layer_map,
        enabled_layers,
        layer_names,
        info: DrawingInfo::new(drawing),
        restroke_paints: sync::Arc::from(restroke_paints.as_slice()),
    })
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
