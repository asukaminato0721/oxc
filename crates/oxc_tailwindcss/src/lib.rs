//! Native Tailwind CSS v4 design-system primitives.
//!
//! This crate deliberately contains no Node or JavaScript bridge. A loaded design system owns a
//! dependency graph for its CSS entrypoint, so long-running callers can invalidate it when any
//! transitive input changes.

mod cache;
mod candidate;
mod canonical;
mod compile;
mod defaults;
mod design_system;
mod loader;
mod property_order;
mod segment;

pub use cache::{CacheKey, DesignSystemCache};
pub use candidate::{
    ArbitraryValue, Candidate, CandidateKind, CandidateValue, Modifier, NamedValue, Variant,
    VariantKind, VariantValue,
};
pub use canonical::CanonicalizeOptions;
pub use compile::{ClassConflict, CompiledRule, Property};
pub use design_system::{
    Compounds, Dependency, DependencyStamp, DesignSystem, LoadOptions, UtilityKind,
    VariantRegistration,
};
pub use loader::LoadError;
