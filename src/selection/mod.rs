//! Block candidate selection — four greedy strategies + topological ordering.

pub(crate) mod ordering;
pub(crate) mod strategies;

pub(crate) use ordering::sel_008_topological_order;
pub(crate) use strategies::{
    sel_002_is_selectable, sel_007_best, sel_greedy, SortStrategy,
};
