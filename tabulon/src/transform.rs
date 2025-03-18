// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Utilities for transformations

use peniko::kurbo::{Affine, Vec2};

/// A direct isometry.
///
/// Direct isometries do not include reflections.
#[derive(Debug, Clone, Copy)]
pub struct DirectIsometry {
    /// Angle in radians to rotate at the origin.
    pub angle: f64,
    /// Displacement from the origin.
    pub displacement: Vec2,
}

impl DirectIsometry {
    /// Make a new `DirectIsometry` from an `angle` and a `displacement`.
    #[inline(always)]
    pub fn new(angle: f64, displacement: Vec2) -> Self {
        Self {
            angle,
            displacement,
        }
    }
}

impl From<DirectIsometry> for Affine {
    #[inline]
    fn from(
        DirectIsometry {
            angle,
            displacement,
        }: DirectIsometry,
    ) -> Self {
        Self::rotate(angle).then_translate(displacement)
    }
}
