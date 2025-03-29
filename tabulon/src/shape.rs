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

extern crate alloc;
use alloc::sync;

use crate::TransformHandle;

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

macro_rules! impl_any_shape_generic_transform {
    ( $self:ident, $transform:ident, $($path_first:ident)|*, $($name:ident)|* ) => {
        match $self {
            // shapes that need converted into path first
            $(AnyShape::$path_first(x) => ($transform * x.to_path(DEFAULT_ACCURACY)).into(),)*
            // shapes that have `impl Mul<...> for Affine`
            $(AnyShape::$name(x) => ($transform * x.clone()).into(),)*
        }
    };
}

impl AnyShape {
    /// Transform shape
    pub fn transform(&self, transform: Affine) -> Self {
        impl_any_shape_generic_transform!(
            self,
            transform,
            // I ran into issues with the `Affine` multiplication implementation
            // on `Arc` in this context, but it behaves correctly after it is
            // converted to a `BezPath` before transforming.
            // Can move this down to the direct impls if/when that is fixed.
            Arc | CircleSegment | Rect | RoundedRect,
            BezPath | Circle | CubicBez | Ellipse | Line | PathSeg | QuadBez
        )
    }

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
            // TODO: do this the cheaper way
            Self::Ellipse(e) => e
                .path_segments(DEFAULT_ACCURACY)
                .fold(f64::INFINITY, |a, b| {
                    a.min(b.nearest(p, DEFAULT_ACCURACY).distance_sq)
                }),
            // TODO: do this the cheaper way
            Self::Arc(e) => e
                .path_segments(DEFAULT_ACCURACY)
                .fold(f64::INFINITY, |a, b| {
                    a.min(b.nearest(p, DEFAULT_ACCURACY).distance_sq)
                }),
            // TODO: do this the cheaper way
            Self::Rect(e) => e
                .path_segments(DEFAULT_ACCURACY)
                .fold(f64::INFINITY, |a, b| {
                    a.min(b.nearest(p, DEFAULT_ACCURACY).distance_sq)
                }),
            // TODO: do this the cheaper way
            Self::RoundedRect(e) => e
                .path_segments(DEFAULT_ACCURACY)
                .fold(f64::INFINITY, |a, b| {
                    a.min(b.nearest(p, DEFAULT_ACCURACY).distance_sq)
                }),
            // TODO: do this the cheaper way
            Self::CircleSegment(e) => e
                .path_segments(DEFAULT_ACCURACY)
                .fold(f64::INFINITY, |a, b| {
                    a.min(b.nearest(p, DEFAULT_ACCURACY).distance_sq)
                }),
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
    pub transform: TransformHandle,
    /// Paint information
    pub paint: FatPaint,
    /// [`AnyShape`]s
    pub subshapes: sync::Arc<[AnyShape]>,
}

impl FatShape {
    /// Get union of subshape bounding boxes.
    pub fn bounding_box(&self) -> Option<Rect> {
        self.subshapes.get(1).map(|s| {
            self.subshapes
                .iter()
                .map(|x| x.bounding_box())
                .fold(s.bounding_box(), |a, x| a.union(x))
        })
    }
}
