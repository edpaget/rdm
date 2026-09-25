//! Review-specific scaffolding for the review workflow tests: where the
//! review sources live, the review-core host wrapper [`Js`], the planted
//! finder/refuter agent, label parsing and survivor lookups. The generic
//! half (failure model, mutant libraries, the recording scripted agent) is
//! `rdm-cli/tests/common/workflow_support.rs`, re-exported here.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use rdm_devtools::workflow::{Host, JsError};
use serde_json::{Value, json};

pub use crate::workflow_support::*;

/// The canonical review core.
pub const REVIEW_LIB: &str = ".claude/workflows/lib/review.mjs";
/// The plan-review driver module (imports the review core).
pub const PLAN_LIB: &str = ".claude/workflows/lib/plan-review.mjs";
/// The standalone review engine.
pub const REVIEW_ENGINE: &str = ".claude/workflows/rdm-wf-review-refute-fix.js";
/// The standalone plan-review engine.
pub const PLAN_ENGINE: &str = ".claude/workflows/rdm-wf-plan-review.js";
/// The engine's shipped (embedded template) copy.
pub const REVIEW_ENGINE_TEMPLATE: &str =
    "rdm-core/src/templates/workflows/rdm-wf-review-refute-fix.js";

impl Lib {
    /// A copy of the review core and the plan-review module with each
    /// `(file, from, to)` edit applied exactly once.
    ///
    /// # Panics
    ///
    /// As for [`Lib::mutant_of`].
    pub fn mutant(name: &str, edits: &[(&str, &str, &str)]) -> Self {
        Self::mutant_of(name, &[REVIEW_LIB, PLAN_LIB], edits)
    }
}

/// A started host with the review core imported.
pub struct Js {
    /// The underlying host (for registering callbacks and raw calls).
    pub host: Host,
    review: Value,
    plan: Option<Value>,
    plan_path: PathBuf,
}

impl Js {
    /// Starts a host and imports `lib`'s review core.
    pub fn open(lib: &Lib) -> Result<Self, Failure> {
        let mut host = Host::start_default()?;
        let review = load(host.import(&lib.path(REVIEW_LIB)))?;
        Ok(Self {
            host,
            review,
            plan: None,
            plan_path: lib.path(PLAN_LIB),
        })
    }

    /// An export of the review core.
    pub fn get(&mut self, name: &str) -> Result<Value, Failure> {
        Ok(self.host.export(&self.review, name)?)
    }

    /// Calls an export of the review core; a JavaScript throw is a failure.
    pub fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Failure> {
        Ok(self.host.call_export(&self.review, name, args)?)
    }

    /// Calls an export of the review core, returning a JavaScript throw as a
    /// value for the scenario to inspect.
    pub fn try_call(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Result<Value, JsError>, Failure> {
        split(self.host.call_export(&self.review, name, args))
    }

    fn plan_module(&mut self) -> Result<Value, Failure> {
        if self.plan.is_none() {
            self.plan = Some(load(self.host.import(&self.plan_path))?);
        }
        Ok(self.plan.clone().unwrap_or(Value::Null))
    }

    /// Calls an export of the plan-review module.
    pub fn plan_call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Failure> {
        let m = self.plan_module()?;
        Ok(self.host.call_export(&m, name, args)?)
    }

    /// Like [`Js::plan_call`], returning a JavaScript throw as a value.
    pub fn plan_try_call(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Result<Value, JsError>, Failure> {
        let m = self.plan_module()?;
        split(self.host.call_export(&m, name, args))
    }

    /// Runs `buildReviewPipeline(mode, deps)(ctx)` with `agent` installed as
    /// the fake agent and a recording `log`.
    pub fn review(
        &mut self,
        mode: &str,
        agent: &Agent,
        ctx: Value,
    ) -> Result<Result<Value, JsError>, Failure> {
        let deps = agent.install(&mut self.host);
        let run = match self.try_call("buildReviewPipeline", vec![json!(mode), deps])? {
            Ok(run) => run,
            Err(e) => return Ok(Err(e)),
        };
        split(self.host.call(&run, vec![ctx]))
    }

    /// [`Js::review`], treating a rejection as a failure.
    pub fn review_ok(&mut self, mode: &str, agent: &Agent, ctx: Value) -> Result<Value, Failure> {
        self.review(mode, agent, ctx)?.map_err(Failure::Js)
    }
}

impl Agent {
    /// The common fixture: finder `find:<mode>:<dim>` returns
    /// `{ findings: findings[dim] or [] }`; refuter `refute:<mode>:<id>`
    /// returns `verdicts[id]` or `{ refuted: false, confidence: 90 }`.
    pub fn planted(findings: Value, verdicts: Value) -> Self {
        Self::scripted(move |call| planted_reply(&findings, &verdicts, call))
    }

    /// Finding ids of every refuter dispatched, in arrival order.
    pub fn refuted_ids(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|c| match parse_label(&c.label) {
                Label::Refute { id, .. } => Some(id),
                _ => None,
            })
            .collect()
    }
}

/// The [`Agent::planted`] reply rule.
pub fn planted_reply(findings: &Value, verdicts: &Value, call: &AgentCall) -> Reply {
    match parse_label(&call.label) {
        Label::Find { dim, .. } => Reply::Value(json!({
            "findings": findings.get(&dim).cloned().unwrap_or_else(|| json!([]))
        })),
        Label::Refute { id, .. } => Reply::Value(
            verdicts
                .get(&id)
                .cloned()
                .unwrap_or_else(|| json!({ "refuted": false, "confidence": 90 })),
        ),
        // No consolidator scripted → the pipeline fails open to singletons,
        // i.e. the pre-consolidation behaviour, so every existing scenario
        // keeps its survivors.
        Label::Consolidate { .. } => Reply::Null,
        Label::Other => Reply::Throw(format!("unexpected agent label: {}", call.label)),
    }
}

/// A parsed agent label.
pub enum Label {
    /// `find:<mode>:<dim>[:retry]`.
    Find {
        /// The dimension key.
        dim: String,
        /// Whether this is the `:retry` attempt.
        retry: bool,
    },
    /// `refute:<mode>:<id…>`.
    Refute {
        /// The finding id (may contain `:`).
        id: String,
    },
    /// `consolidate:<mode>[:retry]`.
    Consolidate {
        /// Whether this is the `:retry` attempt.
        retry: bool,
    },
    /// Anything else.
    Other,
}

/// Splits an agent label into its kind and subject.
pub fn parse_label(label: &str) -> Label {
    let parts: Vec<&str> = label.split(':').collect();
    match parts.as_slice() {
        ["find", _mode, dim] => Label::Find {
            dim: (*dim).to_owned(),
            retry: false,
        },
        ["find", _mode, dim, "retry"] => Label::Find {
            dim: (*dim).to_owned(),
            retry: true,
        },
        ["refute", _mode, rest @ ..] if !rest.is_empty() => Label::Refute { id: rest.join(":") },
        ["consolidate", _mode] => Label::Consolidate { retry: false },
        ["consolidate", _mode, "retry"] => Label::Consolidate { retry: true },
        _ => Label::Other,
    }
}

/// Survivor ids of a review result, in order.
pub fn ids(result: &Value, key: &str) -> Vec<String> {
    result[key]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|f| f["id"].as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The survivor with `id`.
pub fn find<'a>(result: &'a Value, key: &str, id: &str) -> Option<&'a Value> {
    result[key]
        .as_array()
        .and_then(|a| a.iter().find(|f| f["id"] == json!(id)))
}

/// A map from dimension/id to value, for [`Agent::planted`].
pub fn plant(pairs: &[(&str, Value)]) -> Value {
    let map: HashMap<&str, Value> = pairs.iter().cloned().collect();
    json!(map)
}
