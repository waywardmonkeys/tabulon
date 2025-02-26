use crate::graphics_bag::{GraphicsBag, GraphicsItem};
use crate::shape::FatShape;
extern crate alloc;

use alloc::vec::Vec;

impl From<FatShape> for GraphicsItem {
    fn from(s: FatShape) -> Self {
        Self::FatShape(s)
    }
}

/// Render layer
#[derive(Debug, Default)]
pub struct RenderLayer {
    /// Collection of [`GraphicsItem`] indices in z order.
    pub indices: Vec<usize>,
}

impl RenderLayer {
    /// Push a [`GraphicsItem`], returning its index in the bag.
    pub fn push_with_bag(&mut self, bag: &mut GraphicsBag, i: impl Into<GraphicsItem>) -> usize {
        let n = bag.push(i);
        self.indices.push(n);
        n
    }
}
