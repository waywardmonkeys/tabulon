// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    graphics_bag::{GraphicsBag, GraphicsItem, ItemHandle},
    shape::FatShape,
    text::FatText,
};

extern crate alloc;
use alloc::vec::Vec;

impl From<FatShape> for GraphicsItem {
    fn from(s: FatShape) -> Self {
        Self::FatShape(s)
    }
}

impl From<FatText> for GraphicsItem {
    fn from(t: FatText) -> Self {
        Self::FatText(t)
    }
}

/// Render layer.
#[derive(Debug, Default)]
pub struct RenderLayer {
    /// Collection of [`GraphicsItem`] indices in z order.
    pub indices: Vec<ItemHandle>,
}

impl RenderLayer {
    /// Push a [`GraphicsItem`], returning its index in the bag.
    pub fn push_with_bag(
        &mut self,
        bag: &mut GraphicsBag,
        i: impl Into<GraphicsItem>,
    ) -> ItemHandle {
        let n = bag.push(i);
        self.indices.push(n);
        n
    }
}
