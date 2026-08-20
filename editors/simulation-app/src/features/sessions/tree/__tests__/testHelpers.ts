/**
 * Test fixture helper for backend `TreeNode` literals.
 *
 * Re-exports the codegen'd classification so test fixtures stay in lockstep
 * with the backend without a hand-mirrored copy. The backing rules live in
 * `crates/lang/codegen/src/archetype_rules.toml`; regenerate the `.generated.ts`
 * via `cargo run -p sysml-codegen --bin emit-ts-classification` whenever the
 * rules change.
 *
 * Usage:
 *   import { archetypeForKind } from './testHelpers';
 *   { id, name, kind, archetype: archetypeForKind(kind), children: [] }
 */
export { archetypeForKind } from '@/types/element-kind-classification.generated';
