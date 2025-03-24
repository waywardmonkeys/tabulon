// Copyright 2025 the Tabulon Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

extern crate alloc;
use alloc::vec::Vec;

use crate::{shape::FatShape, text::FatText};

/// Items for [`GraphicsBag`].
#[derive(Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "Making FatShape more indirect doesn't help, and there is no other elegant way to handle this."
)]
pub enum GraphicsItem {
    /// See [`FatShape`].
    FatShape(FatShape),
    /// See [`FatText`].
    FatText(FatText),
}

/// Bag of [`GraphicsItem`]s.
#[derive(Debug, Default)]
pub struct GraphicsBag {
    /// [`GraphicsItem`]s in the bag.
    pub items: Vec<GraphicsItem>,
}

impl GraphicsBag {
    /// Push a [`GraphicsItem`], returning its index.
    pub fn push(&mut self, i: impl Into<GraphicsItem>) -> usize {
        let n = self.items.len();
        self.items.push(i.into());
        n
    }

    /// Get an individual [`GraphicsItem`].
    pub fn get(&self, idx: usize) -> Option<&GraphicsItem> {
        self.items.get(idx)
    }
}
