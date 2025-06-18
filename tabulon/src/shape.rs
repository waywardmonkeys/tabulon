// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use peniko::{
    Brush,
    kurbo::{BezPath, Rect, Shape, Stroke},
};

extern crate alloc;
use alloc::sync;

use crate::{PaintHandle, TransformHandle};

/// Paint style for [`FatShape`].
#[derive(Debug, Default, Clone)]
pub struct FatPaint {
    /// Stroke information
    pub stroke: Stroke,
    /// `Brush` for stroke
    pub stroke_paint: Option<Brush>,
    /// `Brush` for fill
    pub fill_paint: Option<Brush>,
}

/// Collection of subshapes with the same transform and paint style.
#[derive(Debug, Default, Clone)]
pub struct FatShape {
    /// Affine transform
    pub transform: TransformHandle,
    /// Paint information
    pub paint: PaintHandle,
    /// Path.
    pub path: sync::Arc<BezPath>,
}

impl FatShape {
    /// Get the bounding box of the path.
    pub fn bounding_box(&self) -> Option<Rect> {
        let mut s = self.path.segments();
        let f = s.next()?;
        Some(
            s.map(|x| x.bounding_box())
                .fold(f.bounding_box(), |a, x| a.union(x)),
        )
    }
}
