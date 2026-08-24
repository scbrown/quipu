//! Individual MCP tool implementations.
//!
//! Split by responsibility so each file stays within the 500-line limit:
//! reads, searches, shape/subscription management, and writes.

mod datasets;
mod graphs;
mod queries;
mod read;
pub(crate) mod search;
mod shapes;
mod write;

pub use datasets::tool_datasets;
pub use graphs::{tool_graph_freeze, tool_graph_list, tool_graph_thaw};
pub use queries::tool_queries;
pub use read::{tool_cord, tool_unravel};
pub use search::{tool_hybrid_search, tool_search};
pub use shapes::{resolve_validation_shapes, tool_shapes, tool_subscriptions, tool_validate};
pub use write::{tool_episode, tool_retract, tool_retract_episode, tool_set};
