//! The pinned SCBench manifest (`bench/scbench/manifest.toml`): every value a
//! result depends on, parsed strictly and validated before anything runs.
//!
//! An `[[arm]]` with `base = "<arm>"` is an ablation variant: it copies a
//! plain arm (one without `base`) and overrides only its rdm `config`
//! ([`ConfigOverrides`], applied to the container's plan repo with
//! `rdm config set`), its `dispatch` flags ([`DispatchFlags`], rendered into
//! the prompt's `@RDM_DISPATCH_FLAGS@`) and its limits. A variant with a
//! `sweep = { setting, values }` expands into one arm per value, named
//! `<name or base>.<setting leaf>-<value>`. [`Manifest::parse`] expands every
//! variant and sweep, so [`Manifest::arms`] holds only concrete arms.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A parsed, validated manifest whose variants and sweeps are expanded into
/// concrete arms.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Manifest {
    /// The pinned SlopCodeBench checkout.
    pub scbench: Pin,
    /// The pinned problem repository and the subset to run.
    pub problems: Problems,
    /// The agent, model and evaluation settings every arm shares.
    pub agent: Agent,
    /// The arms, run in this order, with every variant and sweep expanded
    /// where its `[[arm]]` stands.
    #[serde(rename = "arm")]
    pub arms: Vec<Arm>,
    /// The rdm-equipped image's plan repo settings; required when an arm
    /// selects `image = "rdm"` and by `scbench image`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rdm: Option<Rdm>,
    /// The directory holding the manifest (relative paths resolve against it).
    #[serde(skip)]
    pub dir: PathBuf,
}

/// A git repository pinned to one commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    /// The clone URL (or a local path).
    pub repo: String,
    /// The full 40-character commit SHA.
    pub sha: String,
}

/// The pinned problem repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Problems {
    /// The clone URL (or a local path).
    pub repo: String,
    /// The full 40-character commit SHA.
    pub sha: String,
    /// The problem names to run, in order.
    pub subset: Vec<String>,
    /// Each subset problem's checkpoint count, pinned with [`Problems::sha`]
    /// (the problem's `config.yaml` `checkpoints:`); its keys are exactly
    /// [`Problems::subset`]. It sizes the projected run count and floors the
    /// report's pass-rate denominator.
    pub checkpoints: BTreeMap<String, u64>,
}

/// Agent, model and evaluation settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// SCBench provider name (`claude_code_oauth` or `anthropic`).
    pub provider: String,
    /// SCBench model name.
    pub model: String,
    /// The model definition rdm ships, relative to the manifest's directory
    /// unless absolute.
    pub model_file: String,
    /// SCBench thinking preset.
    pub thinking: String,
    /// The Claude Code version installed in the agent image.
    pub claude_code_version: String,
    /// The evaluation pass policy.
    pub pass_policy: String,
    /// The SCBench environment name.
    pub environment: String,
    /// The agent's step limit.
    pub step_limit: u64,
    /// The agent's cost limit (0 = unlimited).
    pub cost_limit: f64,
    /// The agent's net cost limit (0 = unlimited).
    pub net_cost_limit: f64,
}

/// One concrete arm: a named prompt, run in the stock or the rdm-equipped
/// image.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Arm {
    /// The arm name (`[A-Za-z0-9_.-]+`, not starting with `.`; used as a
    /// directory name).
    pub name: String,
    /// The plain arm this one is a variant of; `None` for a plain arm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The prompt template, relative to the SCBench checkout or, with
    /// [`PromptFrom::Manifest`], to the manifest's directory.
    pub prompt: String,
    /// Where [`Arm::prompt`] lives (default [`PromptFrom::Scbench`]).
    pub prompt_from: PromptFrom,
    /// The agent image the arm runs in (default [`ArmImage::Stock`]).
    pub image: ArmImage,
    /// The treatment group the arm belongs to; when absent it follows
    /// [`Arm::image`] (see [`Arm::group`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<ArmGroup>,
    /// Overrides `[agent] step_limit` for this arm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_limit: Option<u64>,
    /// Overrides `[agent] cost_limit` for this arm (0 = unlimited).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_limit: Option<f64>,
    /// Overrides `[agent] net_cost_limit` for this arm (0 = unlimited).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_cost_limit: Option<f64>,
    /// rdm config keys the wrapper sets in the arm's plan repo (rdm image
    /// only).
    #[serde(skip_serializing_if = "ConfigOverrides::is_empty")]
    pub config: ConfigOverrides,
    /// Flags rendered into the prompt's `@RDM_DISPATCH_FLAGS@` (manifest
    /// prompts only).
    #[serde(skip_serializing_if = "DispatchFlags::is_empty")]
    pub dispatch: DispatchFlags,
}

/// The rdm config keys an arm may override: only keys `rdm config set`
/// accepts that change the lane. It mirrors `rdm.toml`'s own shape
/// (`config.dispatch.verify`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigOverrides {
    /// `max_refutations`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_refutations: Option<u64>,
    /// The `dispatch` table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch: Option<ConfigDispatch>,
}

/// The `config.dispatch` table of an arm.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigDispatch {
    /// `dispatch.verify`: replaces `[rdm] verify` for the arm (a non-empty
    /// single line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<String>,
}

impl ConfigOverrides {
    /// Whether no key is set.
    pub fn is_empty(&self) -> bool {
        self.max_refutations.is_none() && self.verify().is_none()
    }

    /// The `dispatch.verify` override, if set.
    pub fn verify(&self) -> Option<&str> {
        self.dispatch.as_ref().and_then(|d| d.verify.as_deref())
    }

    /// Every set key, as the rdm key and the value passed to
    /// `rdm config set`, in key order.
    pub fn entries(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        if let Some(v) = self.max_refutations {
            out.insert("max_refutations".to_owned(), v.to_string());
        }
        if let Some(v) = self.verify() {
            out.insert("dispatch.verify".to_owned(), v.to_owned());
        }
        out
    }

    fn merge(&mut self, over: &Self) {
        if over.max_refutations.is_some() {
            self.max_refutations = over.max_refutations;
        }
        if let Some(v) = over.verify() {
            self.dispatch = Some(ConfigDispatch {
                verify: Some(v.to_owned()),
            });
        }
    }
}

/// The shipped `rdm:dispatch-phase` flags an arm may pass.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchFlags {
    /// `--max-refutations <n>` (`0` is a value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_refutations: Option<u64>,
    /// `--max-plan-revise <n>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_plan_revise: Option<u64>,
    /// `--max-code-rework <n>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_code_rework: Option<u64>,
    /// `--models <preset>` (`[A-Za-z0-9_-]+`; rdm rejects an unknown preset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<String>,
}

impl DispatchFlags {
    /// Whether no flag is set.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// The flags as arguments, in a fixed order: `--max-plan-revise`,
    /// `--max-code-rework`, `--max-refutations`, `--models`.
    pub fn args(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (flag, value) in [
            (
                "--max-plan-revise",
                self.max_plan_revise.map(|n| n.to_string()),
            ),
            (
                "--max-code-rework",
                self.max_code_rework.map(|n| n.to_string()),
            ),
            (
                "--max-refutations",
                self.max_refutations.map(|n| n.to_string()),
            ),
            ("--models", self.models.clone()),
        ] {
            if let Some(v) = value {
                out.push(flag.to_owned());
                out.push(v);
            }
        }
        out
    }

    fn merge(&mut self, over: &Self) {
        if over.max_refutations.is_some() {
            self.max_refutations = over.max_refutations;
        }
        if over.max_plan_revise.is_some() {
            self.max_plan_revise = over.max_plan_revise;
        }
        if over.max_code_rework.is_some() {
            self.max_code_rework = over.max_code_rework;
        }
        if over.models.is_some() {
            self.models.clone_from(&over.models);
        }
    }
}

/// An arm's fully resolved overrides (inherited from its base plus its
/// own), as the dry-run plan and the run record hold them.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Overrides {
    /// rdm config key -> value as passed to `rdm config set`
    /// (`dispatch.verify` included).
    pub config: BTreeMap<String, String>,
    /// The dispatch flags.
    pub dispatch: DispatchFlags,
    /// The arm's own `step_limit`, if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_limit: Option<u64>,
    /// The arm's own `cost_limit`, if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_limit: Option<f64>,
    /// The arm's own `net_cost_limit`, if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_cost_limit: Option<f64>,
}

/// The settings a `sweep` may vary, one per sweep.
pub const SWEEP_SETTINGS: &[&str] = &[
    "config.max_refutations",
    "config.dispatch.verify",
    "dispatch.max_refutations",
    "dispatch.max_plan_revise",
    "dispatch.max_code_rework",
    "dispatch.models",
    "step_limit",
    "cost_limit",
    "net_cost_limit",
];

/// Where an arm's prompt template lives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptFrom {
    /// In the SCBench checkout, used in place and unrendered.
    #[default]
    Scbench,
    /// rdm's own template beside the manifest: the runner fills its
    /// `@RDM_ROADMAP@`/`@RDM_PROJECT@`/`@RDM_VERIFY@`/`@RDM_DISPATCH_FLAGS@`
    /// placeholders and writes it to
    /// `<out>/<arm>/prompt.jinja` (see [`super::lane::render_prompt`]).
    Manifest,
}

/// The agent limits one arm runs with (SCBench's `cost_limits`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Limits {
    /// The agent's step limit (`--max-turns`).
    pub step_limit: u64,
    /// The agent's cost limit (0 = unlimited).
    pub cost_limit: f64,
    /// The agent's net cost limit (0 = unlimited).
    pub net_cost_limit: f64,
}

impl Agent {
    /// The agent-wide limits every arm inherits.
    pub fn limits(&self) -> Limits {
        Limits {
            step_limit: self.step_limit,
            cost_limit: self.cost_limit,
            net_cost_limit: self.net_cost_limit,
        }
    }
}

impl Arm {
    /// This arm's limits: each one the arm sets, else `agent`'s.
    pub fn limits(&self, agent: &Agent) -> Limits {
        Limits {
            step_limit: self.step_limit.unwrap_or(agent.step_limit),
            cost_limit: self.cost_limit.unwrap_or(agent.cost_limit),
            net_cost_limit: self.net_cost_limit.unwrap_or(agent.net_cost_limit),
        }
    }

    /// This arm's treatment group: the explicit [`Arm::group`], else
    /// [`ArmGroup::Rdm`] for the rdm image and [`ArmGroup::Control`] for the
    /// stock one.
    pub fn group(&self) -> ArmGroup {
        group_for(self.group, self.image)
    }

    /// The verify command this arm runs under, when the arm runs in the rdm
    /// image (the wrapper sets the gate) or its prompt is rendered from the
    /// manifest (the prompt names the command): its `config.dispatch.verify`
    /// when set, else `rdm.verify`. `None` for an upstream-prompt stock
    /// control.
    pub fn verify<'a>(&'a self, rdm: Option<&'a Rdm>) -> Option<&'a str> {
        if self.image == ArmImage::Rdm || self.prompt_from == PromptFrom::Manifest {
            self.config.verify().or(rdm.map(|r| r.verify.as_str()))
        } else {
            None
        }
    }

    /// The arm's fully resolved overrides.
    pub fn overrides(&self) -> Overrides {
        Overrides {
            config: self.config.entries(),
            dispatch: self.dispatch.clone(),
            step_limit: self.step_limit,
            cost_limit: self.cost_limit,
            net_cost_limit: self.net_cost_limit,
        }
    }

    /// The `RDM_SCBENCH_CONFIG` value: every config override except
    /// `dispatch.verify` (which `RDM_SCBENCH_VERIFY` carries) as
    /// newline-separated `key=value` lines in key order; `None` when there
    /// are none.
    pub fn config_lines(&self) -> Option<String> {
        let lines: Vec<String> = self
            .config
            .entries()
            .into_iter()
            .filter(|(k, _)| k != "dispatch.verify")
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    fn apply(&mut self, f: &Fragment) {
        self.config.merge(&f.config);
        self.dispatch.merge(&f.dispatch);
        if f.step_limit.is_some() {
            self.step_limit = f.step_limit;
        }
        if f.cost_limit.is_some() {
            self.cost_limit = f.cost_limit;
        }
        if f.net_cost_limit.is_some() {
            self.net_cost_limit = f.net_cost_limit;
        }
    }
}

/// Whether `name` is a valid arm name: `[A-Za-z0-9_.-]+`, not starting with
/// `.` (so never `.`, `..` or a hidden directory).
pub fn valid_arm_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

// --- The manifest as written, before variants are expanded --------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    scbench: Pin,
    problems: Problems,
    agent: Agent,
    #[serde(rename = "arm", default)]
    arms: Vec<ArmSpec>,
    #[serde(default)]
    rdm: Option<Rdm>,
}

/// One `[[arm]]` as written: a plain arm, a named variant or a sweep.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArmSpec {
    name: Option<String>,
    base: Option<String>,
    prompt: Option<String>,
    prompt_from: Option<PromptFrom>,
    image: Option<ArmImage>,
    group: Option<ArmGroup>,
    step_limit: Option<u64>,
    cost_limit: Option<f64>,
    net_cost_limit: Option<f64>,
    #[serde(default)]
    config: ConfigOverrides,
    #[serde(default)]
    dispatch: DispatchFlags,
    sweep: Option<Sweep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sweep {
    setting: String,
    values: Vec<toml::Value>,
}

/// One set of overrides: what a variant declares, or one sweep value.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fragment {
    #[serde(default)]
    config: ConfigOverrides,
    #[serde(default)]
    dispatch: DispatchFlags,
    step_limit: Option<u64>,
    cost_limit: Option<f64>,
    net_cost_limit: Option<f64>,
}

impl Fragment {
    /// The single-setting fragment `setting = value`, through the same typed
    /// path a hand-written override takes.
    fn of(setting: &str, value: &toml::Value) -> Result<Self, String> {
        let mut v = value.clone();
        for seg in setting.rsplit('.') {
            let mut t = toml::Table::new();
            t.insert(seg.to_owned(), v);
            v = toml::Value::Table(t);
        }
        v.try_into().map_err(|e: toml::de::Error| e.to_string())
    }

    /// Whether this fragment sets `setting` (one of [`SWEEP_SETTINGS`]).
    fn sets(&self, setting: &str) -> bool {
        match setting {
            "config.max_refutations" => self.config.max_refutations.is_some(),
            "config.dispatch.verify" => self.config.verify().is_some(),
            "dispatch.max_refutations" => self.dispatch.max_refutations.is_some(),
            "dispatch.max_plan_revise" => self.dispatch.max_plan_revise.is_some(),
            "dispatch.max_code_rework" => self.dispatch.max_code_rework.is_some(),
            "dispatch.models" => self.dispatch.models.is_some(),
            "step_limit" => self.step_limit.is_some(),
            "cost_limit" => self.cost_limit.is_some(),
            "net_cost_limit" => self.net_cost_limit.is_some(),
            _ => false,
        }
    }
}

/// A sweep value as it appears in a generated arm name: an integer in
/// decimal, a float through `Display` (`2.5`, `0`), a string verbatim.
fn render_value(v: &toml::Value) -> Option<String> {
    match v {
        toml::Value::Integer(i) => Some(i.to_string()),
        toml::Value::Float(f) => Some(f.to_string()),
        toml::Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// The concrete arm a plain `[[arm]]` (one without `base`) writes.
fn plain_arm(spec: &ArmSpec) -> Result<Arm, String> {
    let Some(name) = &spec.name else {
        return Err("arm.name: an [[arm]] without base needs a name".to_owned());
    };
    if spec.sweep.is_some() {
        return Err(format!(
            "arm {name}: a sweep needs base = \"<arm>\" naming the arm it varies"
        ));
    }
    let Some(prompt) = &spec.prompt else {
        return Err(format!("arm.prompt of arm \"{name}\" is required"));
    };
    Ok(Arm {
        name: name.clone(),
        base: None,
        prompt: prompt.clone(),
        prompt_from: spec.prompt_from.unwrap_or_default(),
        image: spec.image.unwrap_or_default(),
        group: spec.group,
        step_limit: spec.step_limit,
        cost_limit: spec.cost_limit,
        net_cost_limit: spec.net_cost_limit,
        config: spec.config.clone(),
        dispatch: spec.dispatch.clone(),
    })
}

/// Expands every `[[arm]]` into concrete arms, in manifest order, and
/// checks the name rule and duplicate names.
fn expand(specs: &[ArmSpec]) -> Result<Vec<Arm>, String> {
    let mut plain: BTreeMap<String, Arm> = BTreeMap::new();
    for spec in specs.iter().filter(|s| s.base.is_none()) {
        let arm = plain_arm(spec)?;
        plain.entry(arm.name.clone()).or_insert(arm);
    }
    let variants: BTreeSet<&str> = specs
        .iter()
        .filter(|s| s.base.is_some())
        .filter_map(|s| s.name.as_deref())
        .collect();

    let mut arms = Vec::new();
    // Why each arm exists, for a duplicate-name error: `None` for an arm
    // written by hand.
    let mut origins: Vec<Option<String>> = Vec::new();
    for spec in specs {
        let Some(base) = &spec.base else {
            arms.push(plain_arm(spec)?);
            origins.push(None);
            continue;
        };
        let label = spec.name.as_deref().unwrap_or(base);
        let fixed: Vec<&str> = [
            ("prompt", spec.prompt.is_some()),
            ("prompt_from", spec.prompt_from.is_some()),
            ("image", spec.image.is_some()),
            ("group", spec.group.is_some()),
        ]
        .into_iter()
        .filter_map(|(f, set)| set.then_some(f))
        .collect();
        if !fixed.is_empty() {
            return Err(format!(
                "arm {label}: a variant (base = {base}) may override only config, dispatch, \
                 step_limit, cost_limit, net_cost_limit; it sets {}",
                fixed.join(", ")
            ));
        }
        let Some(base_arm) = plain.get(base) else {
            if variants.contains(base.as_str()) {
                return Err(format!(
                    "arm {label}: base {base} is itself a variant; derive from a plain arm"
                ));
            }
            return Err(format!(
                "arm {label}: unknown base {base}; plain arms: {}",
                plain.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        };
        let declared = Fragment {
            config: spec.config.clone(),
            dispatch: spec.dispatch.clone(),
            step_limit: spec.step_limit,
            cost_limit: spec.cost_limit,
            net_cost_limit: spec.net_cost_limit,
        };
        let mut arm = base_arm.clone();
        arm.base = Some(base.clone());
        arm.apply(&declared);
        let Some(sweep) = &spec.sweep else {
            let Some(name) = &spec.name else {
                return Err(format!(
                    "arm.name: a variant of {base} needs a name (or a sweep)"
                ));
            };
            arm.name = name.clone();
            arms.push(arm);
            origins.push(None);
            continue;
        };
        let setting = sweep.setting.as_str();
        if !SWEEP_SETTINGS.contains(&setting) {
            return Err(format!(
                "arm {label}: sweep setting \"{setting}\" is not supported; accepted: {}",
                SWEEP_SETTINGS.join(", ")
            ));
        }
        if sweep.values.is_empty() {
            return Err(format!(
                "arm {label}: the sweep over {setting} has no values"
            ));
        }
        if declared.sets(setting) {
            return Err(format!(
                "arm {label}: the sweep over {setting} also fixes {setting}; drop one"
            ));
        }
        let leaf = setting.rsplit('.').next().unwrap_or(setting);
        let prefix = spec.name.as_deref().unwrap_or(base);
        for value in &sweep.values {
            let Some(rendered) = render_value(value) else {
                return Err(format!(
                    "arm {label}: sweep value {value} of {setting} must be a number or a string"
                ));
            };
            let fragment = Fragment::of(setting, value).map_err(|e| {
                format!(
                    "arm {label}: sweep value {value} of {setting}: {}",
                    e.trim()
                )
            })?;
            let name = format!("{prefix}.{leaf}-{rendered}");
            if !valid_arm_name(&name) {
                return Err(format!(
                    "arm {label}: sweep value {value} of {setting} cannot form an arm name \
                     (\"{name}\"); declare named variants instead"
                ));
            }
            let mut generated = arm.clone();
            generated.apply(&fragment);
            generated.name = name;
            arms.push(generated);
            origins.push(Some(format!(
                "generated by the sweep over {setting} of base {base}"
            )));
        }
    }

    let mut seen = BTreeSet::new();
    for (arm, origin) in arms.iter().zip(&origins) {
        if !valid_arm_name(&arm.name) {
            return Err(format!(
                "arm.name must match [A-Za-z0-9_.-]+ and not start with \".\", got \"{}\"",
                arm.name
            ));
        }
        if !seen.insert(arm.name.as_str()) {
            return Err(match origin {
                Some(o) => format!("arm.name \"{}\" is duplicated ({o})", arm.name),
                None => format!("arm.name \"{}\" is duplicated", arm.name),
            });
        }
    }
    Ok(arms)
}

/// The treatment group of an arm with the explicit `group` (if any) that
/// runs in `image`: `group`, else [`ArmGroup::Rdm`] for the rdm image and
/// [`ArmGroup::Control`] for the stock one.
pub fn group_for(group: Option<ArmGroup>, image: ArmImage) -> ArmGroup {
    group.unwrap_or(match image {
        ArmImage::Rdm => ArmGroup::Rdm,
        ArmImage::Stock => ArmGroup::Control,
    })
}

/// The treatment group an arm belongs to, which the ablation comparison keys
/// on. It is distinct from [`ArmImage`]: an rdm-image arm whose prompt never
/// uses rdm is a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArmGroup {
    /// An rdm treatment: the arm drives its work through rdm.
    Rdm,
    /// A control: the arm does not use rdm.
    Control,
}

impl ArmGroup {
    /// The group's name as the manifest and every record spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rdm => "rdm",
            Self::Control => "control",
        }
    }
}

/// Which agent image an arm runs in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArmImage {
    /// SCBench's stock `claude_code` image.
    #[default]
    Stock,
    /// The rdm-equipped image (see [`super::rdm_image`]): the rdm binary and
    /// `plugins/rdm` from the checkout under test, and a plan repo.
    Rdm,
}

/// The `[rdm]` table: where the rdm-equipped image keeps its plan repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rdm {
    /// The plan repo's absolute path inside the agent container; it must lie
    /// outside SCBench's graded directory.
    pub plan_root: String,
    /// The plan repo's default project (`[a-z0-9-]+`).
    pub project: String,
    /// The verify gate command (a non-empty single line): the wrapper sets
    /// it as the plan repo's `dispatch.verify`, and every manifest-rendered
    /// prompt names it (see [`Arm::verify`]).
    pub verify: String,
}

impl Manifest {
    /// Reads, parses and validates the manifest at `path`.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, does not parse, or fails validation; the
    /// message names the offending field.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read manifest {}: {e}", path.display()))?;
        let dir = path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        Self::parse(&text, &dir).map_err(|e| format!("invalid manifest {}: {e}", path.display()))
    }

    /// Parses and validates manifest text whose relative paths resolve
    /// against `dir`, expanding every variant and sweep into concrete arms.
    ///
    /// # Errors
    ///
    /// When the text does not parse, a variant or sweep is malformed, or the
    /// result fails validation; the message names the offending field.
    pub fn parse(text: &str, dir: &Path) -> Result<Self, String> {
        let raw: RawManifest = toml::from_str(text).map_err(|e| e.to_string())?;
        let manifest = Self {
            arms: expand(&raw.arms)?,
            scbench: raw.scbench,
            problems: raw.problems,
            agent: raw.agent,
            rdm: raw.rdm,
            dir: dir.to_path_buf(),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// The `[rdm]` table, which `scbench image` requires.
    ///
    /// # Errors
    ///
    /// When the manifest has no `[rdm]` table.
    pub fn rdm_table(&self) -> Result<&Rdm, String> {
        self.rdm
            .as_ref()
            .ok_or_else(|| "rdm: scbench image needs an [rdm] table".to_owned())
    }

    /// A copy holding only the arms `names` selects, in manifest order
    /// (duplicates collapse); `None` selects every arm.
    ///
    /// # Errors
    ///
    /// When a name is not a manifest arm, or `names` selects none; the
    /// message names the flag (`--arms`), the name and the manifest's arms.
    pub fn select_arms(&self, names: Option<&[String]>) -> Result<Self, String> {
        let Some(names) = names else {
            return Ok(self.clone());
        };
        let known = || {
            self.arms
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        if names.is_empty() {
            return Err(format!("--arms: no arm selected; arms: {}", known()));
        }
        if let Some(unknown) = names
            .iter()
            .find(|n| !self.arms.iter().any(|a| &a.name == *n))
        {
            return Err(format!(
                "--arms: no arm \"{unknown}\" in the manifest; arms: {}",
                known()
            ));
        }
        let mut selected = self.clone();
        selected.arms.retain(|a| names.contains(&a.name));
        Ok(selected)
    }

    /// A copy whose `[problems] subset` holds only the problems `names`
    /// selects, in manifest order (duplicates collapse), with
    /// `checkpoints` narrowed to the same problems; `None` selects every
    /// problem.
    ///
    /// # Errors
    ///
    /// When a name is not in the manifest's `[problems] subset`, or `names`
    /// selects none; the message names the flag (`--problems`), the name and
    /// the subset's problems.
    pub fn select_problems(&self, names: Option<&[String]>) -> Result<Self, String> {
        let Some(names) = names else {
            return Ok(self.clone());
        };
        let known = || self.problems.subset.join(", ");
        if names.is_empty() {
            return Err(format!(
                "--problems: no problem selected; problems: {}",
                known()
            ));
        }
        if let Some(unknown) = names.iter().find(|n| !self.problems.subset.contains(n)) {
            return Err(format!(
                "--problems: no problem \"{unknown}\" in the manifest's [problems] subset; \
                 problems: {}",
                known()
            ));
        }
        let mut selected = self.clone();
        selected.problems.subset.retain(|p| names.contains(p));
        selected
            .problems
            .checkpoints
            .retain(|p, _| names.contains(p));
        Ok(selected)
    }

    /// The absolute path of `arm`'s prompt when it is a
    /// [`PromptFrom::Manifest`] prompt; `None` for an SCBench prompt.
    pub fn manifest_prompt_source(&self, arm: &Arm) -> Option<PathBuf> {
        (arm.prompt_from == PromptFrom::Manifest).then(|| self.dir.join(&arm.prompt))
    }

    /// The model file's absolute source path.
    pub fn model_file_source(&self) -> PathBuf {
        let p = Path::new(&self.agent.model_file);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.dir.join(p)
        }
    }

    fn validate(&self) -> Result<(), String> {
        let strings = [
            ("scbench.repo", &self.scbench.repo),
            ("scbench.sha", &self.scbench.sha),
            ("problems.repo", &self.problems.repo),
            ("problems.sha", &self.problems.sha),
            ("agent.provider", &self.agent.provider),
            ("agent.model", &self.agent.model),
            ("agent.model_file", &self.agent.model_file),
            ("agent.thinking", &self.agent.thinking),
            ("agent.claude_code_version", &self.agent.claude_code_version),
            ("agent.pass_policy", &self.agent.pass_policy),
            ("agent.environment", &self.agent.environment),
        ];
        for (field, value) in strings {
            if value.trim().is_empty() {
                return Err(format!("{field} must not be empty"));
            }
        }
        for (field, sha) in [
            ("scbench.sha", &self.scbench.sha),
            ("problems.sha", &self.problems.sha),
        ] {
            if !is_full_sha(sha) {
                return Err(format!(
                    "{field} must be 40 lowercase hex characters, got \"{sha}\""
                ));
            }
        }
        if self.problems.subset.is_empty() {
            return Err("problems.subset must name at least one problem".to_owned());
        }
        if self.problems.subset.iter().any(|p| p.trim().is_empty()) {
            return Err("problems.subset must not contain an empty name".to_owned());
        }
        for problem in &self.problems.subset {
            match self.problems.checkpoints.get(problem) {
                None => {
                    return Err(format!(
                        "problems.checkpoints has no count for subset problem \"{problem}\""
                    ));
                }
                Some(0) => {
                    return Err(format!(
                        "problems.checkpoints of problem \"{problem}\" must be at least 1"
                    ));
                }
                Some(_) => {}
            }
        }
        if let Some(extra) = self
            .problems
            .checkpoints
            .keys()
            .find(|k| !self.problems.subset.contains(k))
        {
            return Err(format!(
                "problems.checkpoints names \"{extra}\", which is not in problems.subset"
            ));
        }
        if self.arms.is_empty() {
            return Err("arm: the manifest must define at least one [[arm]]".to_owned());
        }
        for arm in &self.arms {
            if arm.prompt.trim().is_empty() {
                return Err(format!(
                    "arm.prompt of arm \"{}\" must not be empty",
                    arm.name
                ));
            }
            if !arm.config.is_empty() && arm.image != ArmImage::Rdm {
                return Err(format!(
                    "arm {}: config overrides need image = \"rdm\" (only the rdm wrapper applies \
                     them); it runs in the stock image",
                    arm.name
                ));
            }
            if let Some(verify) = arm.config.verify()
                && (verify.trim().is_empty() || verify.contains(['\n', '\r']))
            {
                return Err(format!(
                    "arm {}: config.dispatch.verify must be a non-empty single line, got {verify:?}",
                    arm.name
                ));
            }
            if !arm.dispatch.is_empty() && arm.prompt_from != PromptFrom::Manifest {
                return Err(format!(
                    "arm {}: dispatch flags need prompt_from = \"manifest\" (only a manifest \
                     prompt renders @RDM_DISPATCH_FLAGS@)",
                    arm.name
                ));
            }
            if let Some(models) = &arm.dispatch.models
                && (models.is_empty()
                    || !models
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            {
                return Err(format!(
                    "arm {}: dispatch.models must match [A-Za-z0-9_-]+, got \"{models}\"",
                    arm.name
                ));
            }
            if arm.image == ArmImage::Rdm && self.rdm.is_none() {
                return Err(format!(
                    "rdm: arm \"{}\" selects image \"rdm\" but the manifest has no [rdm] table",
                    arm.name
                ));
            }
            if arm.group == Some(ArmGroup::Rdm) && arm.image != ArmImage::Rdm {
                return Err(format!(
                    "arm.group of arm \"{}\" is \"rdm\" but the arm runs in the stock image, which has no rdm",
                    arm.name
                ));
            }
            if arm.prompt_from == PromptFrom::Manifest && self.rdm.is_none() {
                return Err(format!(
                    "arm.prompt_from of arm \"{}\" is \"manifest\" but the manifest has no [rdm] table to render it with",
                    arm.name
                ));
            }
        }
        if let Some(rdm) = &self.rdm {
            if !Path::new(&rdm.plan_root).is_absolute() {
                return Err(format!(
                    "rdm.plan_root must be an absolute path, got \"{}\"",
                    rdm.plan_root
                ));
            }
            if rdm.project.is_empty()
                || !rdm
                    .project
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                return Err(format!(
                    "rdm.project must match [a-z0-9-]+, got \"{}\"",
                    rdm.project
                ));
            }
            if rdm.verify.trim().is_empty() {
                return Err("rdm.verify must be a non-empty command".to_owned());
            }
            if rdm.verify.contains(['\n', '\r']) {
                return Err(format!(
                    "rdm.verify must be a single line, got {:?}",
                    rdm.verify
                ));
            }
        }
        Ok(())
    }
}

fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "af5a9cbc311605b5cceb80200ca9832bdff2d7b1";
    const SHA_B: &str = "9cd9ca3a51c3d3e2a99d2488a25baf73a2204451";

    fn text() -> String {
        format!(
            r#"
[scbench]
repo = "https://example.invalid/scbench"
sha = "{SHA_A}"

[problems]
repo = "https://example.invalid/problems"
sha = "{SHA_B}"
subset = ["mvvault", "xjq"]
checkpoints = {{ mvvault = 6, xjq = 5 }}

[agent]
provider = "claude_code_oauth"
model = "opus-5.5"
model_file = "models/opus-5.5.yaml"
thinking = "xhigh"
claude_code_version = "2.1.322"
pass_policy = "all-cases"
environment = "docker-python3.12-uv"
step_limit = 100
cost_limit = 0
net_cost_limit = 0

[[arm]]
name = "just-solve"
prompt = "configs/prompts/just-solve.jinja"

[[arm]]
name = "anti_slop"
prompt = "configs/prompts/anti_slop.jinja"
"#
        )
    }

    fn parse(t: &str) -> Result<Manifest, String> {
        Manifest::parse(t, Path::new("/m"))
    }

    fn fails_naming(t: &str, field: &str) {
        let e = parse(t).expect_err("must fail");
        assert!(e.contains(field), "error {e:?} should name {field:?}");
    }

    #[test]
    fn valid_manifest_parses_and_resolves_model_file() {
        let m = parse(&text()).expect("valid");
        assert_eq!(m.arms.len(), 2);
        assert_eq!(m.arms[1].name, "anti_slop");
        assert_eq!(m.agent.step_limit, 100);
        assert_eq!(
            m.model_file_source(),
            PathBuf::from("/m/models/opus-5.5.yaml")
        );
    }

    #[test]
    fn absolute_model_file_is_kept() {
        let t = text().replace("\"models/opus-5.5.yaml\"", "\"/abs/x.yaml\"");
        let m = parse(&t).expect("valid");
        assert_eq!(m.model_file_source(), PathBuf::from("/abs/x.yaml"));
    }

    #[test]
    fn non_hex_sha_fails() {
        let bad = SHA_A.replace('a', "g");
        fails_naming(&text().replace(SHA_A, &bad), "scbench.sha");
    }

    #[test]
    fn short_sha_fails() {
        fails_naming(&text().replace(SHA_B, &SHA_B[..39]), "problems.sha");
    }

    #[test]
    fn empty_subset_fails() {
        fails_naming(
            &text().replace(r#"["mvvault", "xjq"]"#, "[]"),
            "problems.subset",
        );
    }

    #[test]
    fn no_arm_fails() {
        let t = text();
        let cut = t.find("[[arm]]").expect("has arms");
        fails_naming(&t[..cut], "arm");
    }

    #[test]
    fn duplicate_arm_fails() {
        fails_naming(
            &text().replace("name = \"anti_slop\"", "name = \"just-solve\""),
            "arm.name",
        );
    }

    #[test]
    fn arm_name_with_slash_fails() {
        fails_naming(
            &text().replace("name = \"anti_slop\"", "name = \"a/b\""),
            "arm.name",
        );
    }

    #[test]
    fn unknown_key_fails() {
        fails_naming(
            &text().replace("thinking = \"xhigh\"", "thinking = \"xhigh\"\nseed = 1"),
            "seed",
        );
    }

    const RDM: &str =
        "\n[rdm]\nplan_root = \"/rdm-plan\"\nproject = \"bench\"\nverify = \"pytest -q\"\n";

    fn with_rdm_arm(t: &str) -> String {
        format!("{t}\n[[arm]]\nname = \"r\"\nprompt = \"p.jinja\"\nimage = \"rdm\"\n")
    }

    #[test]
    fn arm_without_image_is_stock() {
        let m = parse(&text()).expect("valid");
        assert!(m.arms.iter().all(|a| a.image == ArmImage::Stock));
        assert_eq!(m.rdm, None);
    }

    #[test]
    fn rdm_arm_with_rdm_table_parses() {
        let m = parse(&with_rdm_arm(&format!("{RDM}{}", text()))).expect("valid");
        assert_eq!(m.arms[2].image, ArmImage::Rdm);
        let rdm = m.rdm_table().expect("rdm table");
        assert_eq!(rdm.plan_root, "/rdm-plan");
        assert_eq!(rdm.project, "bench");
        assert_eq!(rdm.verify, "pytest -q");
    }

    #[test]
    fn empty_rdm_verify_fails() {
        fails_naming(
            &format!("{}{}", RDM.replace("\"pytest -q\"", "\"  \""), text()),
            "rdm.verify",
        );
    }

    #[test]
    fn multi_line_rdm_verify_fails() {
        fails_naming(
            &format!(
                "{}{}",
                RDM.replace("\"pytest -q\"", "\"pytest -q\\nruff check\""),
                text()
            ),
            "rdm.verify",
        );
    }

    #[test]
    fn missing_rdm_verify_fails() {
        fails_naming(
            &format!("{}{}", RDM.replace("verify = \"pytest -q\"\n", ""), text()),
            "verify",
        );
    }

    #[test]
    fn arm_verify_is_the_bench_gate_for_rdm_image_and_manifest_prompt_arms_only() {
        let m = parse(&with_manifest_prompt_arm(
            &with_rdm_arm(&format!("{RDM}{}", text())),
            "",
        ))
        .expect("valid");
        let rdm = m.rdm.as_ref();
        assert_eq!(m.arms[0].verify(rdm), None, "upstream stock control");
        assert_eq!(m.arms[2].verify(rdm), Some("pytest -q"), "rdm image");
        assert_eq!(m.arms[3].verify(rdm), Some("pytest -q"), "manifest prompt");
    }

    #[test]
    fn unknown_image_fails() {
        fails_naming(
            &text().replace(
                "name = \"anti_slop\"",
                "name = \"anti_slop\"\nimage = \"bogus\"",
            ),
            "image",
        );
    }

    #[test]
    fn rdm_arm_without_rdm_table_fails() {
        fails_naming(&with_rdm_arm(&text()), "rdm");
        let m = parse(&text()).expect("valid");
        assert!(m.rdm_table().expect_err("no table").contains("[rdm]"));
    }

    #[test]
    fn relative_plan_root_fails() {
        fails_naming(
            &format!("{}{}", RDM.replace("/rdm-plan", "rdm-plan"), text()),
            "rdm.plan_root",
        );
    }

    #[test]
    fn empty_rdm_project_fails() {
        fails_naming(
            &format!("{}{}", RDM.replace("\"bench\"", "\"\""), text()),
            "rdm.project",
        );
    }

    #[test]
    fn arm_without_prompt_from_is_scbench() {
        let m = parse(&text()).expect("valid");
        assert!(m.arms.iter().all(|a| a.prompt_from == PromptFrom::Scbench));
        assert_eq!(m.manifest_prompt_source(&m.arms[0]), None);
    }

    fn with_manifest_prompt_arm(t: &str, extra: &str) -> String {
        format!(
            "{t}\n[[arm]]\nname = \"lane\"\nprompt = \"prompts/l.jinja\"\nprompt_from = \"manifest\"\n{extra}"
        )
    }

    #[test]
    fn manifest_prompt_resolves_against_manifest_dir() {
        let m = parse(&with_manifest_prompt_arm(&format!("{RDM}{}", text()), "")).expect("valid");
        assert_eq!(m.arms[2].prompt_from, PromptFrom::Manifest);
        assert_eq!(
            m.manifest_prompt_source(&m.arms[2]),
            Some(PathBuf::from("/m/prompts/l.jinja"))
        );
    }

    #[test]
    fn manifest_prompt_without_rdm_table_fails() {
        fails_naming(&with_manifest_prompt_arm(&text(), ""), "prompt_from");
    }

    #[test]
    fn arm_without_limits_inherits_agent_limits() {
        let m = parse(&text()).expect("valid");
        assert_eq!(m.arms[0].limits(&m.agent), m.agent.limits());
        assert_eq!(m.agent.limits().step_limit, 100);
    }

    #[test]
    fn arm_setting_only_step_limit_keeps_agent_cost_limits() {
        let t = text().replace("cost_limit = 0\nnet", "cost_limit = 3.5\nnet");
        let t = with_manifest_prompt_arm(&format!("{RDM}{t}"), "step_limit = 1000\n");
        let m = parse(&t).expect("valid");
        assert_eq!(
            m.arms[2].limits(&m.agent),
            Limits {
                step_limit: 1000,
                cost_limit: 3.5,
                net_cost_limit: 0.0,
            }
        );
    }

    #[test]
    fn arm_overriding_every_limit_uses_its_own() {
        let t = with_manifest_prompt_arm(
            &format!("{RDM}{}", text()),
            "step_limit = 4321\ncost_limit = 12.5\nnet_cost_limit = 25\n",
        );
        let m = parse(&t).expect("valid");
        assert_eq!(
            m.arms[2].limits(&m.agent),
            Limits {
                step_limit: 4321,
                cost_limit: 12.5,
                net_cost_limit: 25.0,
            }
        );
    }

    #[test]
    fn arm_without_group_derives_it_from_image() {
        let m = parse(&with_rdm_arm(&format!("{RDM}{}", text()))).expect("valid");
        assert!(m.arms.iter().all(|a| a.group.is_none()));
        assert_eq!(m.arms[0].group(), ArmGroup::Control);
        assert_eq!(m.arms[1].group(), ArmGroup::Control);
        assert_eq!(m.arms[2].group(), ArmGroup::Rdm);
    }

    #[test]
    fn group_for_prefers_the_explicit_group_and_names_it_as_serde_does() {
        assert_eq!(group_for(None, ArmImage::Stock), ArmGroup::Control);
        assert_eq!(group_for(None, ArmImage::Rdm), ArmGroup::Rdm);
        assert_eq!(
            group_for(Some(ArmGroup::Control), ArmImage::Rdm),
            ArmGroup::Control
        );
        assert_eq!(
            group_for(Some(ArmGroup::Rdm), ArmImage::Stock),
            ArmGroup::Rdm
        );
        for g in [ArmGroup::Rdm, ArmGroup::Control] {
            assert_eq!(
                serde_json::to_value(g).expect("serialize"),
                g.as_str(),
                "{g:?}"
            );
        }
    }

    #[test]
    fn explicit_control_group_on_rdm_image_is_kept() {
        let t = format!(
            "{}group = \"control\"\n",
            with_rdm_arm(&format!("{RDM}{}", text()))
        );
        let m = parse(&t).expect("valid");
        assert_eq!(m.arms[2].image, ArmImage::Rdm);
        assert_eq!(m.arms[2].group, Some(ArmGroup::Control));
        assert_eq!(m.arms[2].group(), ArmGroup::Control);
    }

    #[test]
    fn rdm_group_on_stock_image_fails() {
        fails_naming(
            &text().replace(
                "name = \"anti_slop\"",
                "name = \"anti_slop\"\ngroup = \"rdm\"",
            ),
            "arm.group",
        );
    }

    #[test]
    fn unknown_group_fails() {
        fails_naming(
            &text().replace(
                "name = \"anti_slop\"",
                "name = \"anti_slop\"\ngroup = \"bogus\"",
            ),
            "group",
        );
    }

    #[test]
    fn checkpoints_parse_per_problem() {
        let m = parse(&text()).expect("valid");
        assert_eq!(
            m.problems.checkpoints,
            BTreeMap::from([("mvvault".to_owned(), 6), ("xjq".to_owned(), 5)])
        );
    }

    #[test]
    fn checkpoints_missing_a_subset_problem_fails() {
        let e =
            parse(&text().replace("mvvault = 6, xjq = 5", "mvvault = 6")).expect_err("must fail");
        assert!(
            e.contains("problems.checkpoints") && e.contains("xjq"),
            "{e}"
        );
    }

    #[test]
    fn checkpoints_naming_a_problem_outside_the_subset_fails() {
        let e = parse(&text().replace("xjq = 5", "xjq = 5, other = 2")).expect_err("must fail");
        assert!(
            e.contains("problems.checkpoints") && e.contains("other"),
            "{e}"
        );
    }

    #[test]
    fn zero_checkpoints_fails() {
        let e = parse(&text().replace("xjq = 5", "xjq = 0")).expect_err("must fail");
        assert!(
            e.contains("problems.checkpoints") && e.contains("xjq"),
            "{e}"
        );
    }

    #[test]
    fn missing_checkpoints_fails() {
        fails_naming(
            &text().replace("checkpoints = { mvvault = 6, xjq = 5 }\n", ""),
            "checkpoints",
        );
    }

    fn arm_names(m: &Manifest) -> Vec<&str> {
        m.arms.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn select_arms_keeps_manifest_order_and_collapses_duplicates() {
        let m = parse(&text()).expect("valid");
        let names = ["anti_slop", "just-solve", "anti_slop"].map(str::to_owned);
        let picked = m.select_arms(Some(&names)).expect("selected");
        assert_eq!(arm_names(&picked), ["just-solve", "anti_slop"]);
        let one = m
            .select_arms(Some(&["anti_slop".to_owned()]))
            .expect("selected");
        assert_eq!(arm_names(&one), ["anti_slop"]);
        assert_eq!(m.select_arms(None).expect("all"), m);
    }

    #[test]
    fn select_arms_refuses_an_unknown_or_empty_selection() {
        let m = parse(&text()).expect("valid");
        let e = m
            .select_arms(Some(&["nope".to_owned()]))
            .expect_err("unknown");
        assert!(
            e.contains("--arms") && e.contains("\"nope\"") && e.contains("just-solve, anti_slop"),
            "{e}"
        );
        let e = m.select_arms(Some(&[])).expect_err("empty");
        assert!(e.contains("--arms"), "{e}");
    }

    #[test]
    fn select_problems_keeps_manifest_order_collapses_duplicates_and_narrows_checkpoints() {
        let m = parse(&text()).expect("valid");
        let names = ["xjq", "mvvault", "xjq"].map(str::to_owned);
        let both = m.select_problems(Some(&names)).expect("selected");
        assert_eq!(both.problems.subset, ["mvvault", "xjq"]);
        assert_eq!(both.problems.checkpoints, m.problems.checkpoints);
        let one = m
            .select_problems(Some(&["xjq".to_owned(), "xjq".to_owned()]))
            .expect("selected");
        assert_eq!(one.problems.subset, ["xjq"]);
        assert_eq!(
            one.problems.checkpoints,
            BTreeMap::from([("xjq".to_owned(), 5)])
        );
        assert_eq!(one.arms, m.arms);
        assert_eq!(m.select_problems(None).expect("all"), m);
    }

    #[test]
    fn select_problems_refuses_an_unknown_or_empty_selection() {
        let m = parse(&text()).expect("valid");
        let e = m
            .select_problems(Some(&["nope".to_owned()]))
            .expect_err("unknown");
        assert_eq!(
            e,
            "--problems: no problem \"nope\" in the manifest's [problems] subset; \
             problems: mvvault, xjq"
        );
        let e = m.select_problems(Some(&[])).expect_err("empty");
        assert_eq!(e, "--problems: no problem selected; problems: mvvault, xjq");
    }

    /// [`text`] with `[rdm]`, a plain rdm-image, manifest-prompt `full-lane`
    /// arm (limits, a config key and a dispatch flag set) and `extra`.
    fn variants(extra: &str) -> String {
        format!(
            "{RDM}{}\n[[arm]]\nname = \"full-lane\"\nprompt = \"prompts/full-lane.jinja\"\n\
             prompt_from = \"manifest\"\nimage = \"rdm\"\ngroup = \"rdm\"\nstep_limit = 1000\n\
             cost_limit = 4\nconfig = {{ max_refutations = 3, dispatch = {{ verify = \"pytest\" }} }}\n\
             dispatch = {{ max_plan_revise = 2 }}\n{extra}",
            text()
        )
    }

    fn arm<'a>(m: &'a Manifest, name: &str) -> &'a Arm {
        m.arms
            .iter()
            .find(|a| a.name == name)
            .unwrap_or_else(|| panic!("no arm {name}: {:?}", arm_names(m)))
    }

    #[test]
    fn a_variant_inherits_its_base_and_overrides_only_what_it_declares() {
        let m = parse(&variants(
            "[[arm]]\nname = \"fl\"\nbase = \"full-lane\"\nstep_limit = 1500\n\
             config.max_refutations = 0\ndispatch = { models = \"fable\" }\n",
        ))
        .expect("valid");
        let base = arm(&m, "full-lane");
        let v = arm(&m, "fl");
        assert_eq!(v.base.as_deref(), Some("full-lane"));
        assert_eq!(base.base, None);
        assert_eq!(
            (&v.prompt, v.prompt_from, v.image, v.group),
            (&base.prompt, base.prompt_from, base.image, base.group)
        );
        assert_eq!(
            (v.step_limit, v.cost_limit, v.net_cost_limit),
            (Some(1500), Some(4.0), None)
        );
        assert_eq!(v.config.max_refutations, Some(0));
        assert_eq!(v.config.verify(), Some("pytest"), "inherited from the base");
        assert_eq!(
            v.dispatch,
            DispatchFlags {
                max_plan_revise: Some(2),
                models: Some("fable".to_owned()),
                ..DispatchFlags::default()
            }
        );
        let o = v.overrides();
        assert_eq!(
            o.config,
            BTreeMap::from([
                ("dispatch.verify".to_owned(), "pytest".to_owned()),
                ("max_refutations".to_owned(), "0".to_owned()),
            ])
        );
        assert_eq!(v.config_lines().as_deref(), Some("max_refutations=0"));
        assert_eq!(v.verify(m.rdm.as_ref()), Some("pytest"));
        assert_eq!(arm(&m, "just-solve").config_lines(), None);
    }

    #[test]
    fn a_variant_is_placed_where_its_spec_stands() {
        let t = variants("[[arm]]\nname = \"fl\"\nbase = \"full-lane\"\n");
        let t = format!("{t}\n[[arm]]\nname = \"last\"\nprompt = \"x.jinja\"\n");
        let m = parse(&t).expect("valid");
        assert_eq!(
            arm_names(&m),
            ["just-solve", "anti_slop", "full-lane", "fl", "last"]
        );
    }

    fn variant_fails(extra: &str, wants: &[&str]) {
        let e = parse(&variants(extra)).expect_err("must fail");
        for want in wants {
            assert!(e.contains(want), "error {e:?} should name {want:?}");
        }
    }

    #[test]
    fn an_unknown_base_fails_naming_it_and_the_plain_arms() {
        variant_fails(
            "[[arm]]\nname = \"v\"\nbase = \"nope\"\n",
            &["nope", "unknown base", "full-lane"],
        );
    }

    #[test]
    fn a_base_that_is_itself_a_variant_fails() {
        variant_fails(
            "[[arm]]\nname = \"v1\"\nbase = \"full-lane\"\n\
             [[arm]]\nname = \"v2\"\nbase = \"v1\"\n",
            &["v1", "itself a variant"],
        );
    }

    #[test]
    fn a_variant_setting_prompt_or_image_fails() {
        variant_fails(
            "[[arm]]\nname = \"v\"\nbase = \"full-lane\"\nprompt = \"y.jinja\"\n",
            &["it sets prompt"],
        );
        variant_fails(
            "[[arm]]\nname = \"v\"\nbase = \"full-lane\"\nimage = \"stock\"\n",
            &["it sets image"],
        );
    }

    #[test]
    fn an_unsupported_config_key_fails() {
        variant_fails(
            "[[arm]]\nname = \"v\"\nbase = \"full-lane\"\nconfig.gates.reviewed = true\n",
            &["gates", "max_refutations"],
        );
        variant_fails(
            "[[arm]]\nname = \"v\"\nbase = \"full-lane\"\nconfig.models.preset = \"fable\"\n",
            &["models"],
        );
    }

    #[test]
    fn an_unsupported_dispatch_key_fails() {
        variant_fails(
            "[[arm]]\nname = \"v\"\nbase = \"full-lane\"\ndispatch.max_turns = 3\n",
            &["max_turns", "max_code_rework"],
        );
    }

    #[test]
    fn a_negative_max_refutations_fails() {
        variant_fails(
            "[[arm]]\nname = \"v\"\nbase = \"full-lane\"\nconfig.max_refutations = -1\n",
            &["max_refutations"],
        );
        variant_fails(
            "[[arm]]\nbase = \"full-lane\"\n\
             sweep = { setting = \"config.max_refutations\", values = [1, -1] }\n",
            &["-1", "config.max_refutations"],
        );
    }

    #[test]
    fn a_multi_line_config_verify_fails() {
        variant_fails(
            "[[arm]]\nname = \"v\"\nbase = \"full-lane\"\n\
             config.dispatch.verify = \"a\\nb\"\n",
            &["config.dispatch.verify"],
        );
    }

    #[test]
    fn config_on_a_stock_arm_and_dispatch_on_an_upstream_prompt_arm_fail() {
        variant_fails(
            "[[arm]]\nname = \"v\"\nbase = \"just-solve\"\nconfig.max_refutations = 1\n",
            &["arm v", "config", "image = \"rdm\""],
        );
        variant_fails(
            "[[arm]]\nname = \"v\"\nbase = \"just-solve\"\ndispatch.models = \"fable\"\n",
            &["arm v", "dispatch", "prompt_from = \"manifest\""],
        );
    }

    #[test]
    fn an_unsafe_models_preset_fails() {
        variant_fails(
            "[[arm]]\nname = \"v\"\nbase = \"full-lane\"\ndispatch.models = \"a b\"\n",
            &["dispatch.models"],
        );
    }

    #[test]
    fn a_sweep_without_base_fails() {
        variant_fails(
            "[[arm]]\nname = \"v\"\nprompt = \"y.jinja\"\n\
             sweep = { setting = \"config.max_refutations\", values = [1] }\n",
            &["arm v", "a sweep needs base"],
        );
    }

    #[test]
    fn a_variant_without_name_or_sweep_fails() {
        variant_fails(
            "[[arm]]\nbase = \"full-lane\"\n",
            &["a variant of full-lane needs a name (or a sweep)"],
        );
    }

    #[test]
    fn a_sweep_value_that_is_not_a_number_or_string_fails() {
        variant_fails(
            "[[arm]]\nbase = \"full-lane\"\n\
             sweep = { setting = \"step_limit\", values = [true] }\n",
            &["step_limit", "must be a number or a string"],
        );
    }

    #[test]
    fn a_sweep_generates_one_arm_per_value_in_place_and_in_order() {
        let m = parse(&variants(
            "[[arm]]\nbase = \"full-lane\"\n\
             sweep = { setting = \"config.max_refutations\", values = [0, 2, 5] }\n\
             [[arm]]\nname = \"after\"\nprompt = \"z.jinja\"\n",
        ))
        .expect("valid");
        assert_eq!(
            arm_names(&m),
            [
                "just-solve",
                "anti_slop",
                "full-lane",
                "full-lane.max_refutations-0",
                "full-lane.max_refutations-2",
                "full-lane.max_refutations-5",
                "after",
            ]
        );
        for n in [0, 2, 5] {
            let a = arm(&m, &format!("full-lane.max_refutations-{n}"));
            assert_eq!(a.base.as_deref(), Some("full-lane"));
            assert_eq!(a.config.max_refutations, Some(n));
            assert_eq!(a.config_lines(), Some(format!("max_refutations={n}")));
            assert_eq!(a.dispatch, arm(&m, "full-lane").dispatch);
        }
        let picked = m
            .select_arms(Some(&["full-lane.max_refutations-0".to_owned()]))
            .expect("selects a generated arm");
        assert_eq!(arm_names(&picked), ["full-lane.max_refutations-0"]);
    }

    #[test]
    fn a_sweep_name_prefix_and_fixed_overrides_apply_to_every_generated_arm() {
        let m = parse(&variants(
            "[[arm]]\nname = \"fl-fable\"\nbase = \"full-lane\"\n\
             dispatch.models = \"fable\"\n\
             sweep = { setting = \"dispatch.max_code_rework\", values = [0, 1] }\n",
        ))
        .expect("valid");
        let a = arm(&m, "fl-fable.max_code_rework-1");
        assert_eq!(a.dispatch.max_code_rework, Some(1));
        assert_eq!(a.dispatch.models.as_deref(), Some("fable"));
        assert!(
            m.arms
                .iter()
                .any(|a| a.name == "fl-fable.max_code_rework-0")
        );
    }

    #[test]
    fn a_float_cost_limit_sweep_renders_its_values() {
        let m = parse(&variants(
            "[[arm]]\nbase = \"full-lane\"\n\
             sweep = { setting = \"cost_limit\", values = [2.5, 0.0, 7] }\n",
        ))
        .expect("valid");
        assert_eq!(arm(&m, "full-lane.cost_limit-2.5").cost_limit, Some(2.5));
        assert_eq!(arm(&m, "full-lane.cost_limit-0").cost_limit, Some(0.0));
        assert_eq!(arm(&m, "full-lane.cost_limit-7").cost_limit, Some(7.0));
    }

    #[test]
    fn an_unsupported_sweep_setting_fails_naming_the_accepted_ones() {
        variant_fails(
            "[[arm]]\nbase = \"full-lane\"\nsweep = { setting = \"gates.reviewed\", values = [true] }\n",
            &[
                "gates.reviewed",
                "config.max_refutations",
                "dispatch.models",
            ],
        );
    }

    #[test]
    fn an_empty_sweep_fails() {
        variant_fails(
            "[[arm]]\nbase = \"full-lane\"\nsweep = { setting = \"step_limit\", values = [] }\n",
            &["step_limit", "no values"],
        );
    }

    #[test]
    fn a_sweep_fixing_its_own_setting_fails() {
        variant_fails(
            "[[arm]]\nbase = \"full-lane\"\nstep_limit = 9\n\
             sweep = { setting = \"step_limit\", values = [1] }\n",
            &["also fixes step_limit"],
        );
    }

    #[test]
    fn a_sweep_value_that_cannot_form_a_name_fails() {
        variant_fails(
            "[[arm]]\nbase = \"full-lane\"\n\
             sweep = { setting = \"config.dispatch.verify\", values = [\"pytest -q\"] }\n",
            &["pytest -q", "cannot form an arm name", "named variants"],
        );
    }

    #[test]
    fn sweeps_whose_generated_names_collide_fail_naming_the_name() {
        variant_fails(
            "[[arm]]\nbase = \"full-lane\"\n\
             sweep = { setting = \"config.max_refutations\", values = [0] }\n\
             [[arm]]\nbase = \"full-lane\"\n\
             sweep = { setting = \"dispatch.max_refutations\", values = [0] }\n",
            &[
                "arm.name \"full-lane.max_refutations-0\" is duplicated",
                "dispatch.max_refutations",
                "base full-lane",
            ],
        );
    }

    #[test]
    fn the_arm_name_rule_allows_dots_but_not_a_leading_one() {
        assert!(valid_arm_name("a.b-1"));
        assert!(valid_arm_name("full-lane.max_refutations-0"));
        for bad in [".hidden", ".", "..", "", "a/b", "a b"] {
            assert!(!valid_arm_name(bad), "{bad:?}");
        }
        let m = parse(&text().replace("name = \"anti_slop\"", "name = \"a.b-1\""))
            .expect("dotted name");
        assert_eq!(m.arms[1].name, "a.b-1");
        fails_naming(
            &text().replace("name = \"anti_slop\"", "name = \".hidden\""),
            "arm.name",
        );
    }

    #[test]
    fn empty_string_field_fails() {
        fails_naming(
            &text().replace("provider = \"claude_code_oauth\"", "provider = \"\""),
            "agent.provider",
        );
    }
}
