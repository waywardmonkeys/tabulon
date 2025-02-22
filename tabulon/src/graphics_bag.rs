extern crate alloc;
use alloc::vec::Vec;

use crate::shape::FatShape;

/// Items for [`GraphicsBag`]
#[derive(Debug)]
pub enum GraphicsItem {
    /// See [`FatShape`]
    FatShape(FatShape),
}

/// Bag of [`GraphicsItem`]s and associated transforms
#[derive(Debug, Default)]
pub struct GraphicsBag {
    items: Vec<GraphicsItem>,
}

impl GraphicsBag {
    /// Push a [`GraphicsItem`], returning its index.
    pub fn push(&mut self, i: impl Into<GraphicsItem>) -> usize {
        let n = self.items.len();
        self.items.push(i.into());
        n
    }

    /// Get an individual [GraphicsItem]
    pub fn get(&self, idx: usize) -> Option<&GraphicsItem> {
        self.items.get(idx)
    }
}
