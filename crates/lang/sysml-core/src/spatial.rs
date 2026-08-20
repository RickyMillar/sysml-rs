//! Spatial and coordinate frame types for SysML v2 spec compliance.
//!
//! Implements the spatial frame model from `SpatialFrames.kerml` and
//! `MeasurementRefCalculations.sysml`:
//! - `SpatialFrame` / `CartesianSpatialFrame`
//! - `CoordinateTransformation` (4x4 homogeneous matrix)
//! - `FrameRegistry` for named frames and transformation lookup
//!
//! These types live in sysml-core because they are spec-level semantic concepts.

use std::collections::HashMap;

use crate::ElementId;

// ---------------------------------------------------------------------------
// Spatial frames
// ---------------------------------------------------------------------------

/// Kind of spatial reference frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpatialFrameKind {
    /// Abstract spatial frame.
    Abstract,
    /// Cartesian (orthogonal basis) spatial frame.
    Cartesian,
}

/// A spatial reference frame (from `SpatialFrames.kerml`).
#[derive(Debug, Clone)]
pub struct SpatialFrame {
    pub id: ElementId,
    pub name: String,
    pub kind: SpatialFrameKind,
}

// ---------------------------------------------------------------------------
// Coordinate transformation (4x4 homogeneous matrix)
// ---------------------------------------------------------------------------

/// A coordinate transformation between two frames.
///
/// Represented as a 4x4 homogeneous transformation matrix:
/// ```text
/// [ R  t ]     R = 3x3 rotation/orientation
/// [ 0  1 ]     t = 3x1 translation (origin offset)
/// ```
///
/// Transforming a point:  p' = M * [p; 1]  (applies rotation + translation)
/// Transforming a vector: v' = R * v        (rotation only, no translation)
#[derive(Debug, Clone)]
pub struct CoordinateTransformation {
    pub source: ElementId,
    pub target: ElementId,
    /// 4x4 homogeneous transformation matrix (row-major).
    pub matrix: [[f64; 4]; 4],
}

impl CoordinateTransformation {
    /// Identity transformation (no rotation, no translation).
    pub fn identity(source: ElementId, target: ElementId) -> Self {
        Self {
            source,
            target,
            matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Create from origin (translation) and basis direction vectors.
    ///
    /// `origin`: [x, y, z] translation from source to target frame origin.
    /// `basis`: 3 column vectors [e1, e2, e3] forming the target frame's
    ///          axes expressed in the source frame.
    pub fn from_origin_and_basis(
        source: ElementId,
        target: ElementId,
        origin: [f64; 3],
        basis: [[f64; 3]; 3],
    ) -> Self {
        Self {
            source,
            target,
            matrix: [
                [basis[0][0], basis[1][0], basis[2][0], origin[0]],
                [basis[0][1], basis[1][1], basis[2][1], origin[1]],
                [basis[0][2], basis[1][2], basis[2][2], origin[2]],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Compose two transformations: self (A→B) then other (B→C) = A→C.
    pub fn compose(&self, other: &Self) -> Self {
        let m = mat4_mult(&self.matrix, &other.matrix);
        Self {
            source: self.source.clone(),
            target: other.target.clone(),
            matrix: m,
        }
    }

    /// Inverse transformation (swap source/target, invert matrix).
    ///
    /// For a rigid-body transform [R t; 0 1], inverse is [R^T  -R^T*t; 0 1].
    pub fn inverse(&self) -> Self {
        // Extract rotation (top-left 3x3) and translation (right column)
        let r = [
            [self.matrix[0][0], self.matrix[0][1], self.matrix[0][2]],
            [self.matrix[1][0], self.matrix[1][1], self.matrix[1][2]],
            [self.matrix[2][0], self.matrix[2][1], self.matrix[2][2]],
        ];
        let t = [self.matrix[0][3], self.matrix[1][3], self.matrix[2][3]];

        // R^T
        let rt = [
            [r[0][0], r[1][0], r[2][0]],
            [r[0][1], r[1][1], r[2][1]],
            [r[0][2], r[1][2], r[2][2]],
        ];

        // -R^T * t
        let neg_rt_t = [
            -(rt[0][0] * t[0] + rt[0][1] * t[1] + rt[0][2] * t[2]),
            -(rt[1][0] * t[0] + rt[1][1] * t[1] + rt[1][2] * t[2]),
            -(rt[2][0] * t[0] + rt[2][1] * t[1] + rt[2][2] * t[2]),
        ];

        Self {
            source: self.target.clone(),
            target: self.source.clone(),
            matrix: [
                [rt[0][0], rt[0][1], rt[0][2], neg_rt_t[0]],
                [rt[1][0], rt[1][1], rt[1][2], neg_rt_t[1]],
                [rt[2][0], rt[2][1], rt[2][2], neg_rt_t[2]],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Transform a 3D point (applies rotation + translation).
    pub fn transform_point(&self, p: [f64; 3]) -> [f64; 3] {
        let m = &self.matrix;
        [
            m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
            m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
            m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
        ]
    }

    /// Transform a 3D vector (rotation only, no translation).
    pub fn transform_vector(&self, v: [f64; 3]) -> [f64; 3] {
        let m = &self.matrix;
        [
            m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
        ]
    }

    /// Extract the 3x3 rotation matrix.
    pub fn rotation(&self) -> [[f64; 3]; 3] {
        [
            [self.matrix[0][0], self.matrix[0][1], self.matrix[0][2]],
            [self.matrix[1][0], self.matrix[1][1], self.matrix[1][2]],
            [self.matrix[2][0], self.matrix[2][1], self.matrix[2][2]],
        ]
    }

    /// Extract the translation vector.
    pub fn translation(&self) -> [f64; 3] {
        [self.matrix[0][3], self.matrix[1][3], self.matrix[2][3]]
    }
}

/// 4x4 matrix multiplication (row-major).
fn mat4_mult(a: &[[f64; 4]; 4], b: &[[f64; 4]; 4]) -> [[f64; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            for k in 0..4 {
                result[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// FrameRegistry
// ---------------------------------------------------------------------------

/// Registry of named spatial frames and transformations between them.
///
/// Provides lookup for coordinate transformations, supporting chained
/// transforms via compose when a direct transform isn't registered.
#[derive(Debug, Clone, Default)]
pub struct FrameRegistry {
    frames: HashMap<String, SpatialFrame>,
    /// Keyed by (source_name, target_name) for lookup.
    transforms: HashMap<(String, String), CoordinateTransformation>,
    default_frame_name: Option<String>,
}

impl FrameRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a spatial frame.
    pub fn register_frame(&mut self, frame: SpatialFrame) {
        self.frames.insert(frame.name.clone(), frame);
    }

    /// Set the default frame (used when no frame argument is provided).
    pub fn set_default_frame(&mut self, name: impl Into<String>) {
        self.default_frame_name = Some(name.into());
    }

    /// Get the default frame, if set.
    pub fn default_frame(&self) -> Option<&SpatialFrame> {
        self.default_frame_name
            .as_ref()
            .and_then(|n| self.frames.get(n))
    }

    /// Get a frame by name.
    pub fn get_frame(&self, name: &str) -> Option<&SpatialFrame> {
        self.frames.get(name)
    }

    /// Register a transformation between two frames.
    ///
    /// Also registers the inverse transformation automatically.
    pub fn register_transform(&mut self, transform: CoordinateTransformation) {
        let src = self
            .frame_name_for_id(&transform.source)
            .unwrap_or_else(|| transform.source.to_string());
        let tgt = self
            .frame_name_for_id(&transform.target)
            .unwrap_or_else(|| transform.target.to_string());
        let inverse = transform.inverse();
        self.transforms
            .insert((src.clone(), tgt.clone()), transform);
        self.transforms.insert((tgt, src), inverse);
    }

    /// Register a transformation using frame names directly.
    pub fn register_transform_named(
        &mut self,
        source_name: impl Into<String>,
        target_name: impl Into<String>,
        transform: CoordinateTransformation,
    ) {
        let src = source_name.into();
        let tgt = target_name.into();
        let inverse = transform.inverse();
        self.transforms
            .insert((src.clone(), tgt.clone()), transform);
        self.transforms.insert((tgt, src), inverse);
    }

    /// Find a direct transformation between two frames by name.
    pub fn find_transform(&self, source: &str, target: &str) -> Option<&CoordinateTransformation> {
        self.transforms.get(&(source.to_owned(), target.to_owned()))
    }

    /// Look up the frame name for an ElementId.
    fn frame_name_for_id(&self, id: &ElementId) -> Option<String> {
        self.frames
            .values()
            .find(|f| f.id == *id)
            .map(|f| f.name.clone())
    }

    /// Total number of registered frames.
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Total number of registered transformations (including inverses).
    pub fn transform_count(&self) -> usize {
        self.transforms.len()
    }
}

// ---------------------------------------------------------------------------
// Graph detection
// ---------------------------------------------------------------------------

/// Walk a `ModelGraph` and build a `FrameRegistry` from
/// `SpatialFrame` / `CartesianSpatialFrame` / `CoordinateFrame` elements.
///
/// The first frame encountered becomes the registry's default frame.
/// Pure graph derivative — natural tracked-query target.
pub fn detect_spatial_frames(graph: &crate::ModelGraph) -> FrameRegistry {
    let mut registry = FrameRegistry::new();
    let mut default_set = false;

    for element in graph.elements.values() {
        let type_name = element
            .get_prop("unresolvedTypeName")
            .and_then(|v| {
                if let crate::Value::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let is_frame = type_name.contains("SpatialFrame")
            || type_name.contains("CoordinateFrame")
            || (element.kind == crate::ElementKind::AttributeUsage && type_name.contains("Frame"));

        if !is_frame {
            continue;
        }

        let name = element
            .name
            .clone()
            .unwrap_or_else(|| element.id.to_string());
        let kind = if type_name.contains("Cartesian") {
            SpatialFrameKind::Cartesian
        } else {
            SpatialFrameKind::Abstract
        };
        registry.register_frame(SpatialFrame {
            id: element.id.clone(),
            name: name.clone(),
            kind,
        });
        if !default_set {
            registry.set_default_frame(&name);
            default_set = true;
        }
    }

    registry
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-10;

    fn assert_vec3_eq(a: [f64; 3], b: [f64; 3]) {
        for i in 0..3 {
            assert!(
                (a[i] - b[i]).abs() < EPS,
                "index {}: {} != {}",
                i,
                a[i],
                b[i]
            );
        }
    }

    fn id(s: &str) -> ElementId {
        ElementId::from_string(s)
    }

    #[test]
    fn test_identity_transform() {
        let t = CoordinateTransformation::identity(id("a"), id("b"));
        assert_vec3_eq(t.transform_point([1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
        assert_vec3_eq(t.transform_vector([4.0, 5.0, 6.0]), [4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_translation_only() {
        let t = CoordinateTransformation::from_origin_and_basis(
            id("a"),
            id("b"),
            [10.0, 20.0, 30.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        );
        // Point is translated
        assert_vec3_eq(t.transform_point([1.0, 2.0, 3.0]), [11.0, 22.0, 33.0]);
        // Vector is NOT translated (rotation only)
        assert_vec3_eq(t.transform_vector([1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_rotation_90_z() {
        // 90° rotation around Z axis: x→y, y→-x
        let t = CoordinateTransformation::from_origin_and_basis(
            id("a"),
            id("b"),
            [0.0, 0.0, 0.0],
            [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        );
        // (1,0,0) → (0,1,0)
        let r = t.transform_point([1.0, 0.0, 0.0]);
        assert_vec3_eq(r, [0.0, 1.0, 0.0]);
        // (0,1,0) → (-1,0,0)
        let r = t.transform_point([0.0, 1.0, 0.0]);
        assert_vec3_eq(r, [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn test_compose() {
        // A→B: translate by (1,0,0)
        let ab = CoordinateTransformation::from_origin_and_basis(
            id("a"),
            id("b"),
            [1.0, 0.0, 0.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        );
        // B→C: translate by (0,2,0)
        let bc = CoordinateTransformation::from_origin_and_basis(
            id("b"),
            id("c"),
            [0.0, 2.0, 0.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        );
        // A→C should translate by (1,2,0)
        let ac = ab.compose(&bc);
        assert_vec3_eq(ac.transform_point([0.0, 0.0, 0.0]), [1.0, 2.0, 0.0]);
    }

    #[test]
    fn test_inverse() {
        let t = CoordinateTransformation::from_origin_and_basis(
            id("a"),
            id("b"),
            [5.0, -3.0, 1.0],
            [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        );
        let inv = t.inverse();
        // T * T^-1 should be identity on a point
        let p = [7.0, -2.0, 4.0];
        let round_trip = inv.transform_point(t.transform_point(p));
        assert_vec3_eq(round_trip, p);
    }

    #[test]
    fn test_inverse_round_trip_vector() {
        let t = CoordinateTransformation::from_origin_and_basis(
            id("a"),
            id("b"),
            [10.0, 20.0, 30.0],
            [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        let inv = t.inverse();
        let v = [3.0, -1.0, 2.0];
        let round_trip = inv.transform_vector(t.transform_vector(v));
        assert_vec3_eq(round_trip, v);
    }

    #[test]
    fn test_rotation_and_translation() {
        // Translate by (1,0,0) then rotate 90° around Z
        let t = CoordinateTransformation::from_origin_and_basis(
            id("a"),
            id("b"),
            [1.0, 0.0, 0.0],
            [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        );
        // Point (1,0,0) → rotate gives (0,1,0) + translate (1,0,0) = (1,1,0)
        let r = t.transform_point([1.0, 0.0, 0.0]);
        assert_vec3_eq(r, [1.0, 1.0, 0.0]);
    }

    #[test]
    fn test_extract_rotation_translation() {
        let t = CoordinateTransformation::from_origin_and_basis(
            id("a"),
            id("b"),
            [1.0, 2.0, 3.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        );
        assert_vec3_eq(t.translation(), [1.0, 2.0, 3.0]);
        let r = t.rotation();
        assert!((r[0][0] - 1.0).abs() < EPS);
        assert!((r[1][1] - 1.0).abs() < EPS);
        assert!((r[2][2] - 1.0).abs() < EPS);
    }

    // ── FrameRegistry tests ──

    #[test]
    fn test_registry_basic() {
        let mut reg = FrameRegistry::new();
        reg.register_frame(SpatialFrame {
            id: id("world"),
            name: "world".into(),
            kind: SpatialFrameKind::Cartesian,
        });
        reg.register_frame(SpatialFrame {
            id: id("body"),
            name: "body".into(),
            kind: SpatialFrameKind::Cartesian,
        });
        reg.set_default_frame("world");

        assert_eq!(reg.frame_count(), 2);
        assert_eq!(reg.default_frame().unwrap().name, "world");
        assert!(reg.get_frame("body").is_some());
    }

    #[test]
    fn test_registry_transform_lookup() {
        let mut reg = FrameRegistry::new();
        reg.register_frame(SpatialFrame {
            id: id("world"),
            name: "world".into(),
            kind: SpatialFrameKind::Cartesian,
        });
        reg.register_frame(SpatialFrame {
            id: id("body"),
            name: "body".into(),
            kind: SpatialFrameKind::Cartesian,
        });

        let t = CoordinateTransformation::from_origin_and_basis(
            id("world"),
            id("body"),
            [5.0, 0.0, 0.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        );
        reg.register_transform_named("world", "body", t);

        // Forward
        let fwd = reg.find_transform("world", "body").unwrap();
        assert_vec3_eq(fwd.transform_point([0.0, 0.0, 0.0]), [5.0, 0.0, 0.0]);

        // Inverse auto-registered
        let inv = reg.find_transform("body", "world").unwrap();
        assert_vec3_eq(inv.transform_point([5.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_registry_transform_count() {
        let mut reg = FrameRegistry::new();
        let t = CoordinateTransformation::identity(id("a"), id("b"));
        reg.register_transform_named("a", "b", t);
        // Forward + inverse
        assert_eq!(reg.transform_count(), 2);
    }
}
