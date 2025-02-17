extern crate alloc;
use alloc::vec::Vec;

use peniko::{
    kurbo::{
        Affine, Arc, BezPath, Circle, CircleSegment, CubicBez, Ellipse, Line, PathSeg, QuadBez,
        Rect, RoundedRect, Stroke,
    },
    Brush,
};
pub use smallvec::SmallVec;

/// Enumeration of Kurbo shapes supported in FatShape
#[derive(Debug, Clone)]
pub enum AnyShape {
    /// [Arc] from Kurbo
    Arc(Arc),
    /// [BezPath] from Kurbo
    BezPath(BezPath),
    /// [Circle] from Kurbo
    Circle(Circle),
    /// [CircleSegment] from Kurbo
    CircleSegment(CircleSegment),
    /// [CubicBez] from Kurbo
    CubicBez(CubicBez),
    /// [Ellipse] from Kurbo
    Ellipse(Ellipse),
    /// [Line] from Kurbo
    Line(Line),
    /// [PathSeg] from Kurbo
    PathSeg(PathSeg),
    /// [QuadBez] from Kurbo
    QuadBez(QuadBez),
    /// [Rect] from Kurbo
    Rect(Rect),
    /// [RoundedRect] from Kurbo
    RoundedRect(RoundedRect),
}

macro_rules! impl_any_shape_from {
    ( $T:ident ) => {
        impl From<$T> for AnyShape {
            fn from(x: $T) -> Self {
                Self::$T(x)
            }
        }
    };
}

impl_any_shape_from!(Arc);
impl_any_shape_from!(BezPath);
impl_any_shape_from!(Circle);
impl_any_shape_from!(CircleSegment);
impl_any_shape_from!(CubicBez);
impl_any_shape_from!(Ellipse);
impl_any_shape_from!(Line);
impl_any_shape_from!(PathSeg);
impl_any_shape_from!(QuadBez);
impl_any_shape_from!(Rect);
impl_any_shape_from!(RoundedRect);

/// Paint style for [FatShape]
#[derive(Debug, Default)]
pub struct FatPaint {
    /// Stroke information
    pub stroke: Stroke,
    /// `Brush` for stroke
    pub stroke_paint: Option<Brush>,
    /// `Brush` for fill
    pub fill_paint: Option<Brush>,
}

/// Collection of subshapes with the same transform and paint style
#[derive(Debug)]
pub struct FatShape {
    /// Affine transform
    pub transform: Affine,
    /// Paint information
    pub paint: FatPaint,
    /// [`AnyShape`]s
    pub subshapes: SmallVec<[AnyShape; 1]>,
}

/// Items for [`DisplayList`]
#[derive(Debug)]
pub enum DisplayItem {
    /// `DisplayItem` is a [`FatShape`]
    FatShape(FatShape),
}

impl From<FatShape> for DisplayItem {
    fn from(s: FatShape) -> Self {
        Self::FatShape(s)
    }
}

/// Simple display list
#[derive(Debug, Default)]
pub struct DisplayList {
    /// Items in DisplayList
    pub items: Vec<DisplayItem>,
}

impl DisplayList {
    /// Push a [`DisplayItem`], returning its index.
    pub fn push(&mut self, i: impl Into<DisplayItem>) -> usize {
        let n = self.items.len();
        self.items.push(i.into());
        n
    }
}
