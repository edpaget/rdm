//! The canonical review decisions the measurement tools replay, reached
//! through the phase-2 workflow binding rather than copied.
//!
//! A measurement of "what would the pipeline have decided" is only worth
//! anything if it replays the SAME rule the live pipeline applies, so the
//! ranking, survival, blocking and AC-gap decisions, the refuter prompt, the
//! dimension table and the non-gating severity set all stay in
//! `.claude/workflows/lib/review.mjs` and are called, never reimplemented.
//! [`ReviewRules`] is the narrow seam: one method per canonical function.
//! [`NodeReviewRules`] implements it with one long-lived
//! [`Host`](crate::workflow::Host) that imports the real `review.mjs`
//! (overridable, which is how a mutant copy is measured).
//!
//! Every caller of [`NodeReviewRules`] therefore needs Node, resolved by
//! [`resolve_node`](crate::workflow::resolve_node); a missing runtime is an
//! actionable error, never a skip. Everything else in [`measure`](super) is
//! pure and runs without a JavaScript runtime.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::jsjson::JsValue;
use crate::workflow::{Host, HostConfig, WorkflowError};

/// Default overall deadline for the review host (a real corpus replays a few
/// thousand calls).
pub const DEFAULT_HOST_TIMEOUT: Duration = Duration::from_secs(600);

/// The repository checkout this crate was built from.
pub fn checkout_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// The canonical review source: `<checkout>/.claude/workflows/lib/review.mjs`.
pub fn default_review_lib() -> PathBuf {
    checkout_root().join(".claude/workflows/lib/review.mjs")
}

/// The canonical review functions a measurement depends on.
pub trait ReviewRules {
    /// `NON_GATING_SEVERITIES`.
    ///
    /// # Errors
    ///
    /// When the canonical source cannot be reached.
    fn non_gating_severities(&mut self) -> Result<Vec<String>, String>;

    /// The always-on (predicate-free) dimension keys of `DIMENSIONS[mode]`.
    ///
    /// # Errors
    ///
    /// When the canonical source cannot be reached.
    fn always_on_dimensions(&mut self, mode: &str) -> Result<Vec<String>, String>;

    /// `rankFindings(findings)`, returned as the permutation of input indices.
    ///
    /// # Errors
    ///
    /// When the canonical function throws or cannot be reached.
    fn rank_findings(&mut self, findings: &[JsValue]) -> Result<Vec<usize>, String>;

    /// `survives(finding, verdict)`.
    ///
    /// # Errors
    ///
    /// When the canonical function throws or cannot be reached.
    fn survives(&mut self, finding: &JsValue, verdict: Option<&JsValue>) -> Result<bool, String>;

    /// `hasBlocking(findings, tier)` (`None`: the default tier).
    ///
    /// # Errors
    ///
    /// When the canonical function throws or cannot be reached.
    fn has_blocking(&mut self, findings: &[JsValue], tier: Option<&str>) -> Result<bool, String>;

    /// `acTableHasGap(acTable)`.
    ///
    /// # Errors
    ///
    /// When the canonical function throws or cannot be reached.
    fn ac_table_has_gap(&mut self, ac_table: Option<&JsValue>) -> Result<bool, String>;

    /// `refutePrompt(mode, dim, finding, { target })`.
    ///
    /// # Errors
    ///
    /// When the canonical function throws or cannot be reached.
    fn refute_prompt(
        &mut self,
        mode: &str,
        dim: &JsValue,
        finding: &JsValue,
        target: &str,
    ) -> Result<String, String>;
}

/// [`ReviewRules`] over the real `review.mjs`, executed by Node.
pub struct NodeReviewRules {
    host: Host,
    module: Value,
    lib: PathBuf,
}

impl std::fmt::Debug for NodeReviewRules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeReviewRules")
            .field("lib", &self.lib)
            .finish_non_exhaustive()
    }
}

fn wf_err(lib: &Path, what: &str, e: &WorkflowError) -> String {
    format!(
        "calling {what} in {} through the workflow binding failed: {e}",
        lib.display()
    )
}

fn as_bool(v: &Value, what: &str) -> Result<bool, String> {
    match v {
        Value::Bool(b) => Ok(*b),
        other => Err(format!("{what} returned a non-boolean: {other}")),
    }
}

impl NodeReviewRules {
    /// Starts a host with the given deadline and imports `lib`.
    ///
    /// # Errors
    ///
    /// When Node cannot be found or started, or the module fails to import.
    pub fn start(lib: &Path, timeout: Duration) -> Result<Self, String> {
        if !lib.is_file() {
            return Err(format!(
                "the canonical review source {} does not exist; pass --review-lib <path to review.mjs>",
                lib.display()
            ));
        }
        let mut host = Host::start(&HostConfig::new().timeout(timeout))
            .map_err(|e| format!("could not start the review host: {e}"))?;
        let module = host
            .import(lib)
            .map_err(|e| wf_err(lib, "the module import", &e))?;
        Ok(Self {
            host,
            module,
            lib: lib.to_owned(),
        })
    }

    /// Starts with the default review source and deadline.
    ///
    /// # Errors
    ///
    /// As for [`NodeReviewRules::start`].
    pub fn start_default() -> Result<Self, String> {
        Self::start(&default_review_lib(), DEFAULT_HOST_TIMEOUT)
    }

    fn export(&mut self, name: &str) -> Result<Value, String> {
        self.host
            .export(&self.module, name)
            .map_err(|e| wf_err(&self.lib, name, &e))
    }

    fn call_json(&mut self, name: &str, args: &[JsValue]) -> Result<Value, String> {
        let f = self.export(name)?;
        let text = JsValue::Array(args.to_vec()).stringify();
        self.host
            .call_with_json_args(&f, &text)
            .map_err(|e| wf_err(&self.lib, name, &e))
    }

    /// Ends the host.
    ///
    /// # Errors
    ///
    /// When the runtime did not exit cleanly.
    pub fn shutdown(self) -> Result<(), String> {
        self.host
            .shutdown()
            .map_err(|e| format!("the review host did not shut down cleanly: {e}"))
    }
}

impl ReviewRules for NodeReviewRules {
    fn non_gating_severities(&mut self) -> Result<Vec<String>, String> {
        let v = self.export("NON_GATING_SEVERITIES")?;
        v.as_array()
            .map(|a| {
                a.iter()
                    .map(|s| s.as_str().map(str::to_owned).unwrap_or_default())
                    .collect()
            })
            .ok_or_else(|| format!("NON_GATING_SEVERITIES is not an array: {v}"))
    }

    fn always_on_dimensions(&mut self, mode: &str) -> Result<Vec<String>, String> {
        let dims = self.export("DIMENSIONS")?;
        Ok(always_on_from_dimensions(&dims, mode))
    }

    fn rank_findings(&mut self, findings: &[JsValue]) -> Result<Vec<usize>, String> {
        let ranked = self.call_json("rankFindings", &[JsValue::Array(findings.to_vec())])?;
        let ranked = ranked
            .as_array()
            .ok_or_else(|| format!("rankFindings returned a non-array: {ranked}"))?;
        permutation_of(findings, ranked)
    }

    fn survives(&mut self, finding: &JsValue, verdict: Option<&JsValue>) -> Result<bool, String> {
        let v = self.call_json(
            "survives",
            &[finding.clone(), verdict.cloned().unwrap_or(JsValue::Null)],
        )?;
        as_bool(&v, "survives")
    }

    fn has_blocking(&mut self, findings: &[JsValue], tier: Option<&str>) -> Result<bool, String> {
        let f = self.export("hasBlocking")?;
        let tier = tier.map_or_else(Host::undefined, |t| json!(t));
        let args = format!(
            "[{},{}]",
            JsValue::Array(findings.to_vec()).stringify(),
            tier
        );
        let v = self
            .host
            .call_with_json_args(&f, &args)
            .map_err(|e| wf_err(&self.lib, "hasBlocking", &e))?;
        as_bool(&v, "hasBlocking")
    }

    fn ac_table_has_gap(&mut self, ac_table: Option<&JsValue>) -> Result<bool, String> {
        let v = self.call_json(
            "acTableHasGap",
            &[ac_table.cloned().unwrap_or(JsValue::Null)],
        )?;
        as_bool(&v, "acTableHasGap")
    }

    fn refute_prompt(
        &mut self,
        mode: &str,
        dim: &JsValue,
        finding: &JsValue,
        target: &str,
    ) -> Result<String, String> {
        let mut context = super::jsjson::JsObject::new();
        context.insert("target", JsValue::String(target.to_owned()));
        let v = self.call_json(
            "refutePrompt",
            &[
                JsValue::String(mode.to_owned()),
                dim.clone(),
                finding.clone(),
                JsValue::Object(context),
            ],
        )?;
        v.as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("refutePrompt returned a non-string: {v}"))
    }
}

/// The predicate-free keys of `DIMENSIONS[mode]` (an entry whose `when` is
/// falsy is always on).
pub fn always_on_from_dimensions(dims: &Value, mode: &str) -> Vec<String> {
    dims.get(mode)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter(|d| !super::sidecar::truthy(d.get("when")))
                .filter_map(|d| d.get("key").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Maps a ranked copy of `input` back to input indices. The canonical ranking
/// is a stable sort, so structurally equal findings keep their relative order
/// and the first unused equal input is the right one.
///
/// # Errors
///
/// When the ranked list is not a permutation of the input.
pub fn permutation_of(input: &[JsValue], ranked: &[Value]) -> Result<Vec<usize>, String> {
    if ranked.len() != input.len() {
        return Err(format!(
            "rankFindings returned {} finding(s) for {} input(s)",
            ranked.len(),
            input.len()
        ));
    }
    let as_json: Vec<Value> = input.iter().map(JsValue::to_json).collect();
    let mut used = vec![false; input.len()];
    let mut out = Vec::with_capacity(input.len());
    for r in ranked {
        let Some(at) = (0..input.len()).find(|&i| !used[i] && as_json[i] == *r) else {
            return Err(format!(
                "rankFindings returned a finding that is not among its inputs: {r}"
            ));
        };
        used[at] = true;
        out.push(at);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permutation_keeps_equal_findings_in_input_order() {
        let a = JsValue::parse(r#"{"id":"x","severity":"blocking"}"#).unwrap_or(JsValue::Null);
        let b = JsValue::parse(r#"{"id":"y","severity":"concern"}"#).unwrap_or(JsValue::Null);
        let input = [b.clone(), a.clone(), a.clone()];
        let ranked = [a.to_json(), a.to_json(), b.to_json()];
        assert_eq!(permutation_of(&input, &ranked), Ok(vec![1, 2, 0]));
        assert!(permutation_of(&input, &ranked[..2]).is_err());
    }

    #[test]
    fn always_on_skips_predicated_dimensions() {
        let dims = json!({"code": [{"key": "ac"}, {"key": "x", "when": true}, {"key": "correctness", "when": null}]});
        assert_eq!(
            always_on_from_dimensions(&dims, "code"),
            ["ac", "correctness"]
        );
        assert!(always_on_from_dimensions(&dims, "plan").is_empty());
    }
}
