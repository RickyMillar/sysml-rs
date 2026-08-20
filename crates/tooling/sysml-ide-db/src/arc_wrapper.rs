//! Macro for generating Arc wrapper types with salsa-compatible PartialEq/Hash.
//!
//! Two modes:
//! - `identity`: PartialEq/Hash use `Arc::ptr_eq` / `Arc::as_ptr` (pointer identity)
//! - `fingerprint`: PartialEq/Hash use a `.fingerprint` field on the inner data

/// Generate `PartialEq`, `Eq`, and `Hash` impls for an Arc wrapper type.
///
/// # Modes
///
/// - **identity**: Uses `Arc::ptr_eq` for equality and `Arc::as_ptr` for hashing.
///   Best for types where pointer identity is sufficient (e.g., memoized results
///   that are only compared within the same salsa revision).
///
/// - **fingerprint**: Uses a `.fingerprint` field on the inner data for equality
///   and hashing. Falls back to `Arc::ptr_eq` first for fast-path. Best for types
///   that need content-based equality across different Arc allocations.
///
/// # Usage
///
/// ```ignore
/// salsa_arc_wrapper!(identity, Outline, Vec<OutlineItem>);
/// salsa_arc_wrapper!(fingerprint, ParseResult, ParseResultData);
/// ```
macro_rules! salsa_arc_wrapper {
    (identity, $wrapper:ident, $inner:ty) => {
        impl PartialEq for $wrapper {
            fn eq(&self, other: &Self) -> bool {
                Arc::ptr_eq(&self.0, &other.0)
            }
        }
        impl Eq for $wrapper {}
        impl Hash for $wrapper {
            fn hash<H: Hasher>(&self, state: &mut H) {
                Arc::as_ptr(&self.0).hash(state);
            }
        }
    };
    (fingerprint, $wrapper:ident, $inner:ty) => {
        impl PartialEq for $wrapper {
            fn eq(&self, other: &Self) -> bool {
                Arc::ptr_eq(&self.0, &other.0) || self.0.fingerprint == other.0.fingerprint
            }
        }
        impl Eq for $wrapper {}
        impl Hash for $wrapper {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.0.fingerprint.hash(state);
            }
        }
    };
}
