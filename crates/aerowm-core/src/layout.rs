use serde::{Deserialize, Serialize};

use crate::geometry::Rect;

/// A layout algorithm takes an available bounding rectangle
/// and the number of windows to arrange, and returns a 
/// vector of rectangles representing the geometry for each window.
pub trait Layout {
    fn apply(&self, area: Rect, num_windows: usize) -> Vec<Rect>;

    /// Stable name used by session persistence to rebuild the layout.
    fn name(&self) -> &'static str;
}

/// Serializable layout description (name + parameters).
///
/// Unknown names fall back to [`LayoutSpec::default`] on restore so a
/// session file from a newer version never breaks startup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LayoutSpec {
    MonadTall { master_ratio: f32, master_count: usize },
    Columns,
    Max,
}

impl Default for LayoutSpec {
    fn default() -> Self {
        LayoutSpec::MonadTall {
            master_ratio: 0.5,
            master_count: 1,
        }
    }
}

impl LayoutSpec {
    /// Canonical spec for a well-known layout name. Returns `None` for
    /// unknown names (caller decides the fallback).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "monad_tall" => Some(LayoutSpec::default()),
            "columns" => Some(LayoutSpec::Columns),
            "max" => Some(LayoutSpec::Max),
            _ => None,
        }
    }

    /// Build the live layout algorithm described by this spec.
    pub fn instantiate(&self) -> Box<dyn Layout> {
        match self {
            LayoutSpec::MonadTall {
                master_ratio,
                master_count,
            } => Box::new(crate::layouts::MonadTall {
                master_ratio: *master_ratio,
                master_count: *master_count,
            }),
            LayoutSpec::Columns => Box::new(crate::layouts::Columns),
            LayoutSpec::Max => Box::new(crate::layouts::Max),
        }
    }
}
