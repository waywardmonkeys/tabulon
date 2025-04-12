// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

extern crate alloc;

use alloc::sync::Arc;

use parley::{Alignment, StyleSet};
use peniko::{
    kurbo::{Size, Vec2},
    Color,
};

use crate::{DirectIsometry, TransformHandle};

/// Reference point where text is attached to an insertion point.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Default)]
pub enum AttachmentPoint {
    /// Top left corner.
    #[default]
    TopLeft = 1,
    /// Center of top edge.
    TopCenter,
    /// Top right corner.
    TopRight,
    /// Middle of the left edge.
    MiddleLeft,
    /// Center of both axes.
    MiddleCenter,
    /// Middle of the right edge.
    MiddleRight,
    /// Bottom left corner.
    BottomLeft,
    /// Center of bottom edge.
    BottomCenter,
    /// Bottom right corner.
    BottomRight,
}

impl AttachmentPoint {
    /// Select a vector that represents the displacement of an [`AttachmentPoint`]
    /// relative to origin with this size.
    pub fn select(
        &self,
        Size {
            width: w,
            height: h,
        }: Size,
    ) -> Vec2 {
        match self {
            Self::TopLeft => Vec2 { x: 0.0, y: 0.0 },
            Self::TopCenter => Vec2 { x: 0.5 * w, y: 0.0 },
            Self::TopRight => Vec2 { x: w, y: 0.0 },
            Self::MiddleLeft => Vec2 { x: 0.0, y: 0.5 * h },
            Self::MiddleCenter => Vec2 {
                x: 0.5 * w,
                y: 0.5 * h,
            },
            Self::MiddleRight => Vec2 { x: w, y: 0.5 * h },
            Self::BottomLeft => Vec2 { x: 0.0, y: h },
            Self::BottomCenter => Vec2 { x: 0.5 * w, y: h },
            Self::BottomRight => Vec2 { x: w, y: h },
        }
    }
}

/// Text item.
#[derive(Debug, Clone)]
pub struct FatText {
    /// Primary transform.
    pub transform: TransformHandle,
    /// Text content.
    pub text: Arc<str>,
    /// Styles for the text.
    pub style: StyleSet<Option<Color>>,
    /// Alignment
    pub alignment: Alignment,
    /// Maximum inline size before line should break.
    pub max_inline_size: Option<f32>,
    /// Insertion transform.
    pub insertion: DirectIsometry,
    /// Reference point for insertion.
    ///
    /// The insertion point is at this corner of the text.
    pub attachment_point: AttachmentPoint,
}
