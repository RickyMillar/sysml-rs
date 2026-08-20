//! # sysml-run-cases
//!
//! Case and verification execution for the SysML v2 runtime.
//!
//! This crate orchestrates higher-level behavioral scenarios: use cases,
//! verification cases, and analysis cases. It coordinates action runners,
//! constraint evaluators, and flow routers to produce verdicts.
//!
//! ## SysML v2 Case Hierarchy
//!
//! ```text
//! CaseDefinition (base)
//! ├── UseCaseDefinition       — actor/subject interaction scenarios
//! ├── AnalysisCaseDefinition  — analysis and computation scenarios
//! └── VerificationCaseDefinition — requirement verification
//!     └── produces VerdictKind (pass | fail | inconclusive | error)
//! ```
//!
//! ## Verification Pipeline
//!
//! ```text
//! 1. Set up subject under verification
//! 2. Execute verification actions (via action runner)
//! 3. Check each requirement (via constraint evaluator)
//! 4. Produce individual requirement verdicts
//! 5. Aggregate to overall verdict
//! ```
//!
//! ## Spec References
//!
//! - `library.systems/Cases.sysml` — Case, objective, actors
//! - `library.systems/VerificationCases.sysml` — VerificationCase, VerdictKind
//! - `library.systems/Requirements.sysml` — RequirementCheck patterns
//! - `SysML-vocab.ttl` — CaseDefinition, VerificationCaseDefinition

mod health;
pub mod trade_study;
mod verdict;
mod requirement;
mod ir;
mod verification;
mod use_case;
mod script;
mod compile;

#[cfg(test)]
pub(crate) mod test_support;

pub use health::requirement_health_diagnostics;
pub use health::verification_health_diagnostics;
pub use trade_study::{
    compile_trade_study, AlternativeIR, ParetoResult, TradeStudyIR, TradeStudyObjective,
    TradeStudyResult,
};
pub use verdict::*;
pub use requirement::*;
pub use ir::*;
pub use verification::*;
pub use use_case::*;
pub use script::*;
pub use compile::*;
