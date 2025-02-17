// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use peniko::{
    kurbo::{
        Affine, Arc, BezPath, Circle, CircleSegment, CubicBez, Ellipse, Line, ParamCurveNearest,
        PathSeg, Point, QuadBez, Rect, RoundedRect, Shape, Stroke, DEFAULT_ACCURACY,
    },
    Brush,
};
pub use smallvec::SmallVec;

#[cfg(all(not(feature = "std"), not(test)))]
use crate::floatfuncs::FloatFuncs;

/// Enumeration of Kurbo shapes supported in `FatShape`.
#[derive(Debug, Clone)]
pub enum AnyShape {
    /// [`Arc`] from Kurbo.
    Arc(Arc),
    /// [`BezPath`] from Kurbo.
    BezPath(BezPath),
    /// [`Circle`] from Kurbo.
    Circle(Circle),
    /// [`CircleSegment`] from Kurbo.
    CircleSegment(CircleSegment),
    /// [`CubicBez`] from Kurbo.
    CubicBez(CubicBez),
    /// [`Ellipse`] from Kurbo.
    Ellipse(Ellipse),
    /// [`Line`] from Kurbo.
    Line(Line),
    /// [`PathSeg`] from Kurbo.
    PathSeg(PathSeg),
    /// [`QuadBez`] from Kurbo.
    QuadBez(QuadBez),
    /// [`Rect`] from Kurbo.
    Rect(Rect),
    /// [`RoundedRect`] from Kurbo.
    RoundedRect(RoundedRect),
}

macro_rules! impl_any_shape_from {
    ( $($T:ident)|* ) => {
        $(impl From<$T> for AnyShape {
            fn from(x: $T) -> Self {
                Self::$T(x)
            }
        })*
    };
}

impl_any_shape_from!(
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

macro_rules! impl_any_shape_fun {
    ( $self:ident, $fun:ident, $($name:ident)|* ) => {
        match $self {
            $(AnyShape::$name(x) => x.$fun(),)*
        }
    };
    ( $self:ident, $fun:ident, $arg:ident, $($name:ident)|* ) => {
        match $self {
            $(AnyShape::$name(x) => x.$fun($arg),)*
        }
    };
}

impl AnyShape {
    /// `true` if given point is inside the shape.
    pub fn contains(&self, p: Point) -> bool {
        impl_any_shape_fun!(
            self,
            contains,
            p,
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
        )
    }

    /// Get bounding box for shape.
    pub fn bounding_box(&self) -> Rect {
        impl_any_shape_fun!(
            self,
            bounding_box,
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
        )
    }

    /// Get distance of a point to nearest point on shape's edge
    ///
    /// When doing this on many shapes to take a minimum or maximum,
    /// it makes more sense to use [`AnyShape::dist_sq`] because it
    /// avoids taking square roots for every distance.
    pub fn dist(&self, p: Point) -> f64 {
        self.dist_sq(p).sqrt()
    }

    /// Get square distance of a point to nearest point on shape's edge.
    pub fn dist_sq(&self, p: Point) -> f64 {
        match self {
            Self::QuadBez(q) => q.nearest(p, DEFAULT_ACCURACY).distance_sq,
            Self::CubicBez(c) => c.nearest(p, DEFAULT_ACCURACY).distance_sq,
            Self::Line(l) => l.nearest(p, DEFAULT_ACCURACY).distance_sq,
            Self::PathSeg(s) => s.nearest(p, DEFAULT_ACCURACY).distance_sq,
            Self::BezPath(b) => b.segments().fold(f64::INFINITY, |a, b| {
                a.min(b.nearest(p, DEFAULT_ACCURACY).distance_sq)
            }),
            Self::Circle(Circle { center, radius }) => {
                (center.distance_squared(p) - radius * radius).abs()
            }
            _ => {
                if self.contains(p) {
                    0.0
                } else {
                    // FIXME: this is obviously wrong
                    f64::INFINITY
                }
            }
        }
    }
}

/// Paint style for [`FatShape`].
#[derive(Debug, Default)]
pub struct FatPaint {
    /// Stroke information
    pub stroke: Stroke,
    /// `Brush` for stroke
    pub stroke_paint: Option<Brush>,
    /// `Brush` for fill
    pub fill_paint: Option<Brush>,
}

/// Collection of subshapes with the same transform and paint style.
#[derive(Debug)]
pub struct FatShape {
    /// Affine transform
    pub transform: Affine,
    /// Paint information
    pub paint: FatPaint,
    /// [`AnyShape`]s
    pub subshapes: SmallVec<[AnyShape; 1]>,
}

impl FatShape {
    /// Get union of subshape bounding boxes.
    pub fn bounding_box(&self) -> Option<Rect> {
        self.subshapes.get(1).map(|s| {
            self.subshapes
                .iter()
                .map(AnyShape::bounding_box)
                .fold(s.bounding_box(), |a, x| a.union(x))
        })
    }

    /// Pick subshapes for point.
    ///
    /// If `paint` has a `fill_paint`, then this filters shapes by [`AnyShape::contains`].
    /// If there is no `fill_paint` but there is a `stroke`, then this is ordered by absolute distance to the edge of the shape.
    ///// TODO: document which subshape types this is implemented for.
    pub fn pick(&self, p: Point, limit: f64) -> SmallVec<[usize; 4]> {
        let tp = self.transform.inverse() * p;
        // FIXME: breaks for nonuniform transforms
        let sq_limit = (self.transform.inverse() * Circle::new(Point::default(), limit))
            .bounding_box()
            .size()
            .to_vec2()
            .hypot2();
        match self.paint {
            FatPaint {
                fill_paint: Some(_),
                ..
            } => self
                .subshapes
                .iter()
                .enumerate()
                .rev()
                .filter_map(|(i, x)| x.contains(tp).then_some(i))
                .collect(),
            FatPaint {
                stroke_paint: Some(_),
                ..
            } => {
                extern crate alloc;
                use alloc::collections::btree_set::BTreeSet;
                use core::cmp::Ordering;
                struct DistIndex {
                    dist: f64,
                    index: usize,
                }
                impl PartialEq for DistIndex {
                    fn eq(&self, b: &Self) -> bool {
                        self.index == b.index && self.dist == b.dist
                    }
                }
                impl Eq for DistIndex {}
                impl PartialOrd for DistIndex {
                    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
                        Some(self.cmp(b))
                    }
                }
                impl Ord for DistIndex {
                    fn cmp(&self, b: &Self) -> Ordering {
                        let o = self.dist.total_cmp(&b.dist);
                        if o.is_eq() {
                            self.index.cmp(&b.index)
                        } else {
                            o
                        }
                    }
                }
                let picks: BTreeSet<DistIndex> = self
                    .subshapes
                    .iter()
                    .enumerate()
                    .rev()
                    .filter_map(|(index, x)| {
                        let dist = x.dist_sq(tp);
                        (dist < sq_limit).then_some(DistIndex { dist, index })
                    })
                    .collect();
                picks.iter().map(|x| x.index).collect()
            }
            _ => SmallVec::new(),
        }
    }
}
