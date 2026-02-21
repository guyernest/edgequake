//! Schema proposal types — re-exported from edgequake-core.
//!
//! The canonical type definitions live in [`edgequake_core::schema`] to avoid
//! circular dependencies (edgequake-schema depends on edgequake-core, not the
//! reverse). This module re-exports them for convenience.

pub use edgequake_core::schema::{
    EntityTypeProposal, RelationTypeProposal, SchemaProposal, SchemaStatus,
};
