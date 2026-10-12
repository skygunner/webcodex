//! Shared filesystem inspection and Git checkpoint implementations.

pub mod file_read_normalize;
pub mod file_read_range;
pub mod path_policy;
pub mod project_context;
pub mod project_overview;
pub mod project_recipe;

#[cfg(test)]
mod project_recipe_tests;
#[cfg(feature = "workspace-checkpoints")]
pub mod workspace_checkpoint;
