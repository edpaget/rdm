//! Agent configuration generation for AI coding assistants.
//!
//! Generates platform-specific instruction files that teach AI agents
//! how to interact with `rdm` via its CLI.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Target platform for agent configuration output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Claude Code (`CLAUDE.md`)
    Claude,
    /// Cross-agent standard (`AGENTS.md`)
    AgentsMd,
    /// Cursor IDE (`.cursor/rules/rdm.mdc`)
    Cursor,
    /// GitHub Copilot (`.github/copilot-instructions.md`)
    Copilot,
    /// Pi coding agent (`.pi/AGENTS.md`)
    Pi,
}

impl Platform {
    /// Returns the conventional file path for this platform's instruction file.
    pub fn conventional_path(&self) -> &'static str {
        match self {
            Platform::Claude => "CLAUDE.md",
            Platform::AgentsMd => "AGENTS.md",
            Platform::Cursor => ".cursor/rules/rdm.mdc",
            Platform::Copilot => ".github/copilot-instructions.md",
            Platform::Pi => ".pi/AGENTS.md",
        }
    }

    /// Returns the user-level base directory for this platform.
    ///
    /// This is the directory that plays the same role as `--out` but for
    /// user-global configuration. The instruction file will be written at
    /// `user_level_dir() / conventional_path()`, just as `--out` writes to
    /// `out / conventional_path()`.
    ///
    /// | Platform   | Directory        |
    /// |------------|------------------|
    /// | Claude     | `~/.claude/`     |
    /// | AgentsMd   | `~/.claude/`     |
    /// | Cursor     | `~/`             |
    /// | Copilot    | `~/`             |
    /// | Pi         | `~/.pi/agent`    |
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be determined.
    pub fn user_level_dir(&self) -> Result<PathBuf, String> {
        let home = home_dir()?;
        let dir = match self {
            // conventional_path is "CLAUDE.md" / "AGENTS.md" — flat file, so base is ~/.claude/
            Platform::Claude | Platform::AgentsMd => home.join(".claude"),
            // conventional_path is ".cursor/rules/rdm.mdc" — includes subdirs
            Platform::Cursor => home,
            // conventional_path is ".github/copilot-instructions.md" — includes subdir
            Platform::Copilot => home,
            // Pi loads AGENTS.md from ~/.pi/agent/ (note: not symmetric with project path)
            Platform::Pi => home.join(".pi/agent"),
        };
        Ok(dir)
    }

    /// Returns the absolute path to the user-level instruction file for this platform.
    ///
    /// For most platforms this is `user_level_dir().join(conventional_path())`,
    /// but Pi breaks the symmetry: its project-local path is `.pi/AGENTS.md`
    /// while its user-global path is `~/.pi/agent/AGENTS.md` (note the extra
    /// `agent/` segment). This method hides that asymmetry so callers don't
    /// need to special-case Pi.
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be determined.
    pub fn user_level_instruction_path(&self) -> Result<PathBuf, String> {
        match self {
            Platform::Pi => Ok(home_dir()?.join(".pi/agent/AGENTS.md")),
            _ => Ok(self.user_level_dir()?.join(self.conventional_path())),
        }
    }

    /// Returns the relative skills directory inside a project root.
    ///
    /// | Platform   | Subdir          |
    /// |------------|-----------------|
    /// | Claude     | `.claude/skills`|
    /// | Pi         | `.pi/skills`    |
    /// | others     | `None`          |
    ///
    /// Returns `None` for platforms that don't follow the Agent Skills standard.
    pub fn project_skills_subdir(&self) -> Option<&'static str> {
        match self {
            Platform::Claude => Some(".claude/skills"),
            Platform::Pi => Some(".pi/skills"),
            _ => None,
        }
    }

    /// Returns the user-level skills directory for this platform.
    ///
    /// | Platform   | Directory                |
    /// |------------|--------------------------|
    /// | Claude     | `~/.claude/skills`       |
    /// | Pi         | `~/.pi/agent/skills`     |
    ///
    /// # Errors
    ///
    /// Returns an error if the platform doesn't support skills, or if the
    /// home directory cannot be determined.
    pub fn user_level_skills_dir(&self) -> Result<PathBuf, String> {
        let home = home_dir()?;
        match self {
            Platform::Claude => Ok(home.join(".claude").join("skills")),
            Platform::Pi => Ok(home.join(".pi").join("agent").join("skills")),
            other => Err(format!("--skills is not supported for platform '{other}'")),
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Platform::Claude => write!(f, "claude"),
            Platform::AgentsMd => write!(f, "agents-md"),
            Platform::Cursor => write!(f, "cursor"),
            Platform::Copilot => write!(f, "copilot"),
            Platform::Pi => write!(f, "pi"),
        }
    }
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" => Ok(Platform::Claude),
            "agents-md" => Ok(Platform::AgentsMd),
            "cursor" => Ok(Platform::Cursor),
            "copilot" => Ok(Platform::Copilot),
            "pi" => Ok(Platform::Pi),
            other => Err(format!(
                "unknown platform '{other}'; expected one of: claude, agents-md, cursor, copilot, pi"
            )),
        }
    }
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "cannot determine home directory (HOME not set)".to_string())
}

/// Options for generating agent configuration.
pub struct AgentConfigOptions {
    /// Target platform.
    pub platform: Platform,
    /// Project name to embed in examples. If `None`, uses `<PROJECT>` placeholder.
    pub project: Option<String>,
    /// Optional path to a principles file to reference in generated output.
    pub principles_file: Option<String>,
    /// When `true`, generate instructions referencing MCP tool calls instead of CLI commands.
    pub mcp: bool,
}

/// Generates agent configuration content for the given options.
///
/// Returns a string containing platform-formatted instructions for
/// interacting with `rdm` via its CLI.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::{AgentConfigOptions, Platform, generate_agent_config};
///
/// let content = generate_agent_config(&AgentConfigOptions {
///     platform: Platform::AgentsMd,
///     project: Some("myproj".to_string()),
///     principles_file: None,
///     mcp: false,
/// });
/// assert!(content.contains("--project myproj"));
/// ```
pub fn generate_agent_config(opts: &AgentConfigOptions) -> String {
    let instructions = if opts.mcp {
        agent_instructions_mcp(opts.project.as_deref(), opts.principles_file.as_deref())
    } else {
        agent_instructions(opts.project.as_deref(), opts.principles_file.as_deref())
    };

    match opts.platform {
        Platform::Cursor => {
            format!(
                "---\ndescription: Instructions for using rdm to manage project roadmaps\nglobs:\n---\n\n{instructions}"
            )
        }
        _ => instructions,
    }
}

fn proj_flag_str(project: Option<&str>) -> String {
    match project {
        Some(name) => format!("--project {name}"),
        None => "--project <PROJECT>".to_string(),
    }
}

/// Generates the core instruction content shared across all platforms.
fn agent_instructions(project: Option<&str>, principles_file: Option<&str>) -> String {
    let proj_flag = proj_flag_str(project);
    let principles = principles_file
        .map(|p| format!("\n\n{}", section_principles(p)))
        .unwrap_or_default();
    include_str!("templates/instructions-cli.md")
        .replace("{proj_flag}", &proj_flag)
        .replace("\n{principles}", &principles)
}

/// A generated skill file with its relative path and content.
pub struct SkillFile {
    /// Relative path within the output directory (e.g., "rdm-roadmap/SKILL.md").
    pub relative_path: &'static str,
    /// The full content of the skill file.
    pub content: String,
}

/// A generated Workflow-tool script with its relative path and content.
///
/// Unlike [`SkillFile`], workflow content is emitted verbatim: there is no
/// project/principles substitution pass, because BOTH environment axes the
/// scripts depend on are RUNTIME arguments the caller supplies — the rdm
/// executable (`rdmBin`, required and fail-closed) and the plan project
/// (`project`, optional and applied only to project-scoped subcommands). The
/// emitted bytes are therefore correct, unmodified, in any consumer repo, and
/// byte-identity with this repo's own `.claude/workflows/` is a CONSEQUENCE of
/// that design rather than a limitation. The scripts are also the
/// already-stamped output of `scripts/gen-workflow-review.sh`. See
/// `docs/workflow-schemas.md` § "Environment args: `rdmBin` and `project`".
pub struct WorkflowFile {
    /// Relative path within `.claude/workflows/` (e.g., "rdm-wf-dispatch-phase.js").
    pub relative_path: &'static str,
    /// The full, unmodified content of the workflow script.
    pub content: &'static str,
}

/// A generated Claude Code custom-agent definition, with its relative path
/// and content.
///
/// Modeled on [`WorkflowFile`]: agent definitions are emitted verbatim, with
/// no project/principles substitution pass, since a `name`/`tools`
/// frontmatter definition has no per-project or per-executable content to
/// substitute.
pub struct AgentFile {
    /// Relative path within `.claude/agents/` (e.g., "rdm-mechanical.md").
    pub relative_path: &'static str,
    /// The full, unmodified content of the agent definition.
    pub content: &'static str,
}

/// Options for generating skill definition files.
pub struct SkillOptions {
    /// Project name to embed in skill CLI invocations.
    pub project: Option<String>,
    /// Optional path to a principles file to reference.
    pub principles_file: Option<String>,
    /// When `true`, generate skills referencing MCP tool calls instead of CLI commands.
    pub mcp: bool,
}

/// Generates Claude Code skill definition files.
///
/// Returns a vector of [`SkillFile`]s, each containing the relative path
/// and content for a skill definition. Skills are reusable agent behaviors
/// triggered by slash commands in Claude Code.
///
/// Both the `mcp: false` (CLI) and `mcp: true` (MCP) branches return the
/// same 11 skills, identified by `relative_path` — a cli/mcp skill-name
/// parity test (`generate_skills_cli_mcp_name_parity`) asserts this holds so
/// a future one-sided addition fails CI.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::{SkillOptions, generate_skills};
///
/// let skills = generate_skills(&SkillOptions {
///     project: Some("myproj".to_string()),
///     principles_file: None,
///     mcp: false,
/// });
/// assert_eq!(skills.len(), 11);
/// assert!(skills[0].content.contains("--project myproj"));
///
/// let mcp_skills = generate_skills(&SkillOptions {
///     project: Some("myproj".to_string()),
///     principles_file: None,
///     mcp: true,
/// });
/// assert_eq!(mcp_skills.len(), 11);
/// ```
pub fn generate_skills(opts: &SkillOptions) -> Vec<SkillFile> {
    let principles_note = opts.principles_file.as_deref().map(skill_principles_note);
    if opts.mcp {
        let proj = proj_param_str(opts.project.as_deref());
        let proj_flag = proj_flag_str(opts.project.as_deref());
        vec![
            skill_roadmap_mcp(&proj, principles_note.as_deref()),
            skill_do_mcp(&proj, principles_note.as_deref()),
            skill_review_mcp(&proj, principles_note.as_deref()),
            skill_document_mcp(&proj, principles_note.as_deref()),
            skill_estimate_mcp(&proj, principles_note.as_deref()),
            skill_dispatch_phase_mcp(&proj, principles_note.as_deref()),
            skill_autopilot_mcp(&proj, principles_note.as_deref()),
            skill_land_mcp(&proj, principles_note.as_deref()),
            skill_revise_mcp(&proj, principles_note.as_deref()),
            skill_plan_review_mcp(&proj, principles_note.as_deref()),
            skill_backlog_mcp(&proj, &proj_flag, principles_note.as_deref()),
        ]
    } else {
        let proj_flag = proj_flag_str(opts.project.as_deref());
        vec![
            skill_roadmap(&proj_flag, principles_note.as_deref()),
            skill_do(&proj_flag, principles_note.as_deref()),
            skill_review(&proj_flag, principles_note.as_deref()),
            skill_document(&proj_flag, principles_note.as_deref()),
            skill_estimate(&proj_flag, principles_note.as_deref()),
            skill_dispatch_phase(&proj_flag, principles_note.as_deref()),
            skill_autopilot(&proj_flag, principles_note.as_deref()),
            skill_land(&proj_flag, principles_note.as_deref()),
            skill_revise(&proj_flag, principles_note.as_deref()),
            skill_plan_review(&proj_flag, principles_note.as_deref()),
            skill_backlog(&proj_flag, principles_note.as_deref()),
        ]
    }
}

/// The single canonical list of shipped engine names.
///
/// One entry per engine, pairing the emitted `relative_path` with the
/// `include_str!` of the template it is emitted from. The two must live in the
/// same entry because `include_str!` takes a string literal and cannot be given
/// a `const`/variable path — so "named in exactly one place" means one table
/// entry per engine, not one literal.
///
/// Only the TWO engines rdm actually distributes appear here. The four
/// local-only engines (`rdm-wf-backlog`, `rdm-wf-document`, `rdm-wf-estimate`,
/// `rdm-wf-plan-review`) are deliberately unshipped — adding them would change
/// emitted bytes and expand the distribution boundary. The emitted-file-count
/// test and the rustdoc example on [`generate_workflows`] deliberately keep
/// their OWN independent literals rather than reading this table, so they stay a real check on what
/// rdm ships instead of asserting the table equals itself.
const SHIPPED_WORKFLOWS: [(&str, &str); 2] = [
    (
        "rdm-wf-dispatch-phase.js",
        include_str!("templates/workflows/rdm-wf-dispatch-phase.js"),
    ),
    (
        "rdm-wf-review-refute-fix.js",
        include_str!("templates/workflows/rdm-wf-review-refute-fix.js"),
    ),
];

/// Returns the autonomous-lane Workflow-tool scripts that ship alongside the
/// Claude Code skills.
///
/// These are the already-stamped `.claude/workflows/*.js` consumers from this
/// repo's own dogfood setup (`rdm-wf-dispatch-phase.js`,
/// `rdm-wf-review-refute-fix.js`), embedded verbatim via `include_str!` and returned
/// unmodified — there is no project/principles substitution, unlike
/// [`generate_skills`]'s `render_skill` pass. This is a separate emission
/// surface from `generate_skills`/`SkillFile`, not folded into it: the two
/// counts (skills vs. workflows) are independent and should not be summed
/// when asserting either one.
///
/// The scripts name no particular rdm executable and no particular rdm
/// project: both arrive as runtime arguments (`rdmBin`, required and
/// fail-closed; `project`, optional and applied only to project-scoped
/// subcommands), so the emitted bytes work unmodified in an arbitrary
/// downstream target repo. `scripts/verify-agent-config-distribution.sh` § 7
/// gates that claim by emitting into a hermetic non-rdm, non-Rust fixture and
/// executing the emitted engines' pipeline logic and built commands there.
/// `lib/*.mjs` (the canonical source modules the scripts are stamped from) is
/// deliberately not shipped here: there is no regeneration script that travels
/// downstream to consume it.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::generate_workflows;
///
/// let workflows = generate_workflows();
/// assert_eq!(workflows.len(), 2);
/// assert_eq!(workflows[0].relative_path, "rdm-wf-dispatch-phase.js");
/// assert_eq!(workflows[1].relative_path, "rdm-wf-review-refute-fix.js");
/// ```
pub fn generate_workflows() -> Vec<WorkflowFile> {
    SHIPPED_WORKFLOWS
        .iter()
        .map(|(relative_path, content)| WorkflowFile {
            relative_path,
            content,
        })
        .collect()
}

/// The single canonical list of shipped Claude Code custom-agent
/// definitions, mirroring [`SHIPPED_WORKFLOWS`]'s "named in exactly one
/// place" shape.
///
/// Currently one entry: `rdm-mechanical`, the mechanical-transcription agent
/// definition the four local-only Workflow scripts (`rdm-wf-backlog.js`,
/// `rdm-wf-document.js`, `rdm-wf-estimate.js`, `rdm-wf-plan-review.js`)
/// resolve against via `agentType`. None of the three *distributed*
/// workflows (`rdm-wf-dispatch-phase.js`, `rdm-wf-review-refute-fix.js`)
/// reference it yet — this table exists so a downstream tree has somewhere
/// for such a reference to resolve, in advance of one being added. See
/// `docs/workflow-schemas.md` § "agentType / effort options spike".
const SHIPPED_AGENTS: [(&str, &str); 1] = [(
    "rdm-mechanical.md",
    include_str!("templates/agents/rdm-mechanical.md"),
)];

/// Returns the Claude Code custom-agent definitions that ship alongside the
/// skills and Workflow-tool scripts.
///
/// This is a separate emission surface from [`generate_skills`] and
/// [`generate_workflows`] — its own count should not be summed with either.
/// Agent definitions are embedded verbatim via `include_str!` and returned
/// unmodified, with no project/principles substitution, exactly like
/// [`generate_workflows`].
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::generate_agents;
///
/// let agents = generate_agents();
/// assert_eq!(agents.len(), 1);
/// assert_eq!(agents[0].relative_path, "rdm-mechanical.md");
/// ```
pub fn generate_agents() -> Vec<AgentFile> {
    SHIPPED_AGENTS
        .iter()
        .map(|(relative_path, content)| AgentFile {
            relative_path,
            content,
        })
        .collect()
}

/// A previously-emitted `.claude/workflows/` file that a newer emission may
/// supersede.
///
/// Two supersession shapes share this table, and [`resolve_superseded_workflows`]
/// handles both with the same matching/removal logic:
///
/// - **Renamed**: `successor: Some(new_name)` — the file at `name` was
///   replaced by a differently-named current template (the motivating case
///   for this mechanism).
/// - **Retired outright**: `successor: None` — the file at `name` has no
///   current template of any name (e.g. the historical `autopilot.js`,
///   retired by the `prose-autopilot-orchestration` roadmap before this
///   table existed).
///
/// `successor` is metadata for readers of the table; the removal decision
/// in [`resolve_superseded_workflows`] only ever consults `name` and
/// `fingerprints`, so it treats both shapes identically.
pub struct SupersededWorkflow {
    /// The relative filename (a bare filename — no path separators, no
    /// `..`, not absolute) this entry may remove, as it was previously
    /// emitted directly under `.claude/workflows/`.
    pub name: &'static str,
    /// Known SHA-256 hex digests of every previously-emitted body for
    /// `name`. A candidate file is only ever removed when its current
    /// on-disk content hashes to one of these — content the user has since
    /// edited, or content this table doesn't recognize, is left in place.
    pub fingerprints: &'static [&'static str],
    /// The current template's relative path this entry was renamed to, or
    /// `None` if the file was retired outright with no successor. See the
    /// type-level doc for how the two shapes differ.
    pub successor: Option<&'static str>,
}

/// The production superseded-workflow table consulted by `rdm agent-config`'s
/// cleanup step.
///
/// Populated by phase 7 of the `project-agnostic-lane` roadmap, which prefixed
/// every `.claude/workflows/` engine with `rdm-wf-` so a listing entry can no
/// longer be mistaken for its identically-named `rdm-*` skill front door. It
/// carries both supersession shapes:
///
/// - **Renamed** — `dispatch-phase.js` and `review-refute-fix.js`, the two
///   engines rdm actually ships, each pointing at its `rdm-wf-` successor.
/// - **Retired outright** — `autopilot.js`, which had no successor to point at
///   and no other cleanup path. It was a shipped template until
///   `prose-autopilot-orchestration` phase 3 replaced the JS drive loop with
///   the prose `rdm-autopilot` skill, so any repo that ran
///   `rdm agent-config claude --skills` before that still carries an orphan
///   engine downstream. Nothing references it, so the emitted-skill
///   invocation-resolution check cannot see it — only this table can remove it.
///   `lib/autopilot.mjs` is deliberately absent: `lib/*.mjs` was never shipped.
///
/// Each entry's fingerprints are the SHA-256 digests of **every** body that
/// path ever held in this repo's history, so a downstream copy emitted by any
/// past release is recognized. A file whose content matches none of them is a
/// user edit and is left in place.
pub const SUPERSEDED_WORKFLOWS: &[SupersededWorkflow] = &[
    SupersededWorkflow {
        name: "dispatch-phase.js",
        fingerprints: &[
            "05c1b5d2500abd165b9f21a93510a08091766dcc73e7f2b1edd1cbcc02bbd1f1",
            "1c7ba2f947b6200fa202649f1ca8fa0ed0bc1ee651e284979366605293f71981",
            "21d2e635986074af50a2239fd821a93f579a1c8a0a90bc0d264feb1ca3a734b5",
            "25ff8ad00197e2253030398101e4f1f8947c60782757c14b44adfb377a854c88",
            "2c0fa8cc03984096aeb64dd0b37ae2275b40214e83519aea35bc74bae2ea4b95",
            "3840e75d0f153b8cf4627467339680c6ae502a112f2b1dba743173b26b569ad1",
            "3b6d05bf3523782c36bedb6e60ce6cbc0f1ee2f656c6ffc1f950d31da27ce908",
            "3db2eeec8abddff336a091e8985ffbdf11323fa60c612d322561ce1dbad5f200",
            "47289be1e85c63ad823326db1e6c4b34ff52f63c0c338de420faa0f141a5a8ca",
            "5ae366dfe172c6a892d68a528e7bc486624c89016843e077ee25712f3a7a6e79",
            "69789faa4cc19ecffc8b12db424a4c583e45b09764c8aeb716e15c75965a0c89",
            "802148dec6ceceb5252e7d94aec95997d4833d9f42c0f75abdf6880c04aa0c2b",
            "8095b4fa080a97a9b98a7a09e2e6b005eef5b8f853ccedb2bf69e410cdb3d16f",
            "8e9940c156803e57a2abfc4ff068674a6740d4115e98a735e187eb5d8911cd4e",
            "9b710124944dbf4dbb9d507fd22983ed957c39398280de542cefc2af628260e0",
            "acc8b40b21779025ede8c7618fdb0f742fa7081e8ba6e1ef8fb446990e8175fe",
            "cec6d000bbcb4c4817d5205fcfd08c97e6caf87dcda9b9010c56354c90e9ee50",
            "cf9cd60a9f0a24189b0ae0a35488bbb9ef4c22513d73603ab9d59053c662b89a",
            "e7644f1718c9f6690cd8136bbf668c26cc19fd7b6a2a93fd97add25a27604522",
            "ef95333938832a4623b14160875bced07420cc14319970c85c69de6a88377999",
        ],
        successor: Some("rdm-wf-dispatch-phase.js"),
    },
    SupersededWorkflow {
        name: "review-refute-fix.js",
        fingerprints: &[
            "032ffd9ea22dee0eeef54ea8433b9bf25955174bf564094e65f33e80ba72229a",
            "156ba2037bfda1783b49b5f4e97b9246b7c9c6087f7df938348767a071301225",
            "1b6013abfddb26b0754ce9db2f5678112cb91c69582f5789769bb9c609c99d9f",
            "25ddd34570ae748009c1779cb2581c0bcd296ad80fbfca1b6d1003f1e8091a85",
            "45038c9836573df5a63cd70e92fa0063073e4520fe1d4083415d469b52cd660b",
            "4ac6868da30dd942e5cf007c7d674dd5389a7070e074d136ca02933aa32b7dad",
            "5a041fb01eeca74b8a0211570ab62f55253437a4e9b3a2c72483811a89fd9468",
            "6e74cbe6b5eb302f7266bd57b16e55053a6d712c4ac61e4087ebb5b2baf8ef84",
            "83e6702b13f60914d5c6c9c4de701949935cb0c9adc36565c508eb5a7bf2ed5c",
            "882cd12b9032cd6e8ee0dd75f4bfa2f692fdb5345517086a182ada73cea756e3",
            "a325c63adacc64ee9594ae26cfe6f1217e33a4f755efe75dfe210d063043bf26",
            "abafeb94cecb9cf89d8d810590ad2608b4df6956266c9c8022010c17636cf8c9",
            "c97bc7ded2498f6bfd839d72b3476fd898660f1c2b21f5d19eb64dcbf725ef3e",
            "d0bfdb5568c455c98f1a939952521c91bceef14ad625620a8f6652b3e8abcf56",
            "d4089dad134aecc4532089c8ae4b3bef740accb3aad300db2ac92d814ab05f69",
            "d74eb6dd0ab57609ae5780bfa8f7e14d1c5c9727dbd4a71688fbe3b852987df5",
            "dfb0d66c7be2228cc08e3899bc8b81fdeeb4330c94951be35491b2758ce30993",
            "e038557330dec51effb4908ba790e3e2904701201a45b5639477e9633d34ab55",
        ],
        successor: Some("rdm-wf-review-refute-fix.js"),
    },
    SupersededWorkflow {
        name: "autopilot.js",
        fingerprints: &[
            "21ec3a6f3c16dbeb658cfcdab953de769052ed6698cefc117bcb142422ace262",
            "5b4013a2d565a45a6bf18cf245168d55bacf96dd34362a8247a40de09e913282",
            "63845c0c766c3dece2008036929294c3c604caea637592eb40c85d277737d11d",
            "6cd5dff20806aa5eba84e3a2de27889fa4d027d819bf26fc681e175fad5abd6e",
            "817c8ecde65ba7d614620927cc86bc9743b10511b4a85e3249ede7d0324be7e4",
            "8c9a9d3449cd1fa4ed3ecf4af40c9b70ac8981b650b260ceeae2d9204342b2e4",
            "95138b284199fa36950054336c89ad346aa332463b99372bbc75f632ecaa3155",
            "de7bad05b6695be2d1375c2f67d0f111dba30a99dcf9c78afa8f3f6a2006c845",
        ],
        successor: None,
    },
];

/// The outcome of resolving one [`SupersededWorkflow`] table entry against
/// an output directory.
///
/// A table entry with no same-named file present in the directory produces
/// no outcome at all — it is simply absent from
/// [`resolve_superseded_workflows`]'s returned `Vec`, distinct from every
/// variant below.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupersededOutcome {
    /// The candidate file's content matched a known fingerprint and was
    /// removed from disk.
    Removed {
        /// The removed file's path.
        path: PathBuf,
    },
    /// A same-named file exists but was left in place: its content matched
    /// no known fingerprint, or it was not a plain readable file (e.g. a
    /// directory or symlink).
    SkippedModified {
        /// The file left in place.
        path: PathBuf,
    },
    /// The candidate file's content matched a known fingerprint, but the
    /// removal itself failed (e.g. a permission error). Reported, never
    /// treated as fatal by the caller.
    Failed {
        /// The file that could not be removed.
        path: PathBuf,
        /// A human-readable description of why removal failed.
        error: String,
    },
    /// A table entry's `name` was rejected as an unsafe (non-bare) filename
    /// — it contained a path separator, a `..` component, was absolute, or
    /// was empty (see [`is_bare_filename`]) — so no filesystem path was ever
    /// constructed or touched for it. Reported like every other skip, per
    /// this mechanism's "report every removal and every skip" contract,
    /// carrying only the offending table-supplied name rather than a
    /// resolved path, since resolving one would defeat the point of
    /// rejecting it.
    InvalidName {
        /// The rejected table entry's `name`, verbatim.
        name: &'static str,
    },
}

/// Computes the lowercase hex-encoded SHA-256 digest of `bytes`.
///
/// Shared by [`SUPERSEDED_WORKFLOWS`]-style fingerprint tables (computed
/// once, offline, when a file is superseded) and by
/// [`resolve_superseded_workflows`] (computed at cleanup time against each
/// candidate file's current on-disk content).
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Returns `true` when `name` is safe to join onto a directory: a single
/// non-empty, non-`..`, non-root path component. Rejects absolute paths,
/// `..` traversal, and any nested separator, so a caller can never resolve
/// outside the directory it started from.
fn is_bare_filename(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

/// Resolves `table` against `workflows_dir`, returning a removed/skipped/
/// failed outcome for every table entry with a same-named file present.
///
/// This is the decision-and-removal logic behind `rdm agent-config`'s
/// superseded-workflow cleanup. For each entry in `table`, it looks for a
/// file directly inside `workflows_dir` whose name is exactly
/// `entry.name` — no directory listing, no globbing, so a file not named in
/// the table is structurally unreachable — and:
///
/// - removes it and reports [`SupersededOutcome::Removed`], if its content
///   hashes to one of `entry.fingerprints`;
/// - leaves it in place and reports [`SupersededOutcome::SkippedModified`],
///   if a same-named file exists but its content matches no known
///   fingerprint, or it isn't a plain file (a directory, a symlink, or
///   otherwise unreadable);
/// - leaves it in place and reports [`SupersededOutcome::Failed`], if the
///   fingerprint matched but the removal call itself failed (e.g. a
///   permission error);
/// - reports [`SupersededOutcome::InvalidName`] and never touches the
///   filesystem at all for that entry, if `entry.name` was rejected as
///   unsafe (see [`is_bare_filename`]) — this is a reported skip, not a
///   silent drop, even though (unlike every other variant) it carries the
///   table-supplied name rather than a resolved path;
/// - produces no outcome at all, if no file exists at that path (including
///   when `workflows_dir` itself doesn't exist).
///
/// If two entries in `table` share the same `name` (a malformed table —
/// the shipped [`SUPERSEDED_WORKFLOWS`] never does this), entries are
/// resolved in order: once the first matching entry removes the file, the
/// second entry's candidate is already absent and produces no outcome. This
/// is deterministic but not a promised feature of the API — a well-formed
/// table has unique names.
///
/// This function is infallible (`Vec`, not `Result`): a removal failure is
/// captured as [`SupersededOutcome::Failed`] rather than propagated, so a
/// caller can never have a superseded-cleanup failure abort an otherwise
/// successful emit.
pub fn resolve_superseded_workflows(
    workflows_dir: &Path,
    table: &[SupersededWorkflow],
) -> Vec<SupersededOutcome> {
    let mut outcomes = Vec::new();
    for entry in table {
        if !is_bare_filename(entry.name) {
            // Reported, never silently dropped: no path is ever resolved
            // for a rejected name, so this carries the raw table name
            // rather than a joined `PathBuf`.
            outcomes.push(SupersededOutcome::InvalidName { name: entry.name });
            continue;
        }
        let candidate = workflows_dir.join(entry.name);

        // `symlink_metadata` never follows the final symlink component, so a
        // symlink at this name is classified by its own type (not a plain
        // file) rather than by whatever it points at — this keeps removal
        // decisions scoped to `workflows_dir`'s own contents.
        let metadata = match std::fs::symlink_metadata(&candidate) {
            Ok(m) => m,
            Err(_) => continue, // absent — no outcome
        };
        if !metadata.is_file() {
            outcomes.push(SupersededOutcome::SkippedModified { path: candidate });
            continue;
        }

        let contents = match std::fs::read(&candidate) {
            Ok(c) => c,
            Err(_) => {
                outcomes.push(SupersededOutcome::SkippedModified { path: candidate });
                continue;
            }
        };
        let digest = sha256_hex(&contents);
        if entry.fingerprints.contains(&digest.as_str()) {
            match std::fs::remove_file(&candidate) {
                Ok(()) => outcomes.push(SupersededOutcome::Removed { path: candidate }),
                Err(e) => outcomes.push(SupersededOutcome::Failed {
                    path: candidate,
                    error: e.to_string(),
                }),
            }
        } else {
            outcomes.push(SupersededOutcome::SkippedModified { path: candidate });
        }
    }
    outcomes
}

// --- Plugin-layout emission -------------------------------------------------
//
// A second, parallel emission surface that packages the SAME skills and
// workflow engines as an installable Claude Code plugin tree:
//
//     <plugin-root>/
//       .claude-plugin/plugin.json
//       skills/<name>/SKILL.md          (11)
//       workflows/rdm-wf-*.js           (2)
//
// Everything below runs as a POST-processing pass over `generate_skills`'s and
// `generate_workflows`' already-rendered output. Nothing here is reachable
// from `generate_skills`/`render_skill` or from any `templates/` file, which
// is what makes raw `--skills` invariance structural rather than merely
// tested. The naming decisions implemented here are recorded in
// `docs/plugin-distribution.md`.

/// The Claude Code plugin name, and therefore the namespace the runtime
/// prefixes onto every skill and workflow the plugin ships (`rdm:<name>`).
///
/// This is the vendor name and is deliberately not configurable.
pub const PLUGIN_NAME: &str = "rdm";

/// The `description` field of the emitted plugin manifest.
///
/// Deliberately a dedicated literal rather than `env!("CARGO_PKG_DESCRIPTION")`:
/// that one describes the `rdm-core` library ("Core library for rdm: …"),
/// which is the wrong thing to show a user browsing installable plugins.
const PLUGIN_DESCRIPTION: &str = "rdm's planning lane for Claude Code: skills for creating roadmaps, implementing and \
     reviewing phases and tasks, and landing finished work, plus the Workflow engines they \
     dispatch.";

/// Maps each raw emitted skill directory name onto its plugin-mode name, in
/// [`generate_skills`] emission order.
///
/// Plugin mode drops the hand-rolled `rdm-` prefix because the plugin
/// namespace already supplies that disambiguation — `rdm:roadmap` rather than
/// `rdm:rdm-roadmap` (`docs/plugin-distribution.md`, Decision 1).
///
/// The table is the ONLY thing that decides whether an `rdm-`-prefixed token
/// is renamed. That is what keeps the rename exact as well as total: engine
/// names (`rdm-wf-dispatch-phase`, `rdm-wf-estimate`), the `rdm-mechanical`
/// agent type, and prose tokens like `rdm-next`/`rdm-side` all share the
/// prefix but are absent from the table, so [`rewrite_skill_names`] copies
/// them through untouched.
const PLUGIN_SKILL_NAMES: [(&str, &str); 11] = [
    ("rdm-roadmap", "roadmap"),
    ("rdm-do", "do"),
    ("rdm-review", "review"),
    ("rdm-document", "document"),
    ("rdm-estimate", "estimate"),
    ("rdm-dispatch-phase", "dispatch-phase"),
    ("rdm-autopilot", "autopilot"),
    ("rdm-land", "land"),
    ("rdm-revise", "revise"),
    ("rdm-plan-review", "plan-review"),
    ("rdm-backlog", "backlog"),
];

/// The plugin-mode note appended to every emitted skill body that needs an
/// `rdmBin` argument.
///
/// A plugin-installed shim has no repo-local `./target/debug/rdm` to assume,
/// so it must resolve the binary at runtime
/// (`docs/plugin-distribution.md`, Decision 4).
const PLUGIN_RDM_BIN_NOTE: &str = "\n## Resolving `rdmBin` (plugin install)\n\nThis skill was installed from the `rdm` plugin, so there is no repo-local build path to assume. Resolve the `rdmBin` argument in this order and use the first that exists:\n\n1. an explicitly supplied `--rdm-bin <path>`;\n2. the `RDM_BIN` environment variable;\n3. a plain `rdm` on `PATH`.\n\nIf none resolves, stop and report: `rdm binary not found. Install rdm, then set RDM_BIN=/path/to/rdm, put rdm on your PATH, or pass --rdm-bin /path/to/rdm.` Never guess a path, and never invoke a workflow without one.\n";

/// A generated plugin-tree file with its plugin-root-relative path and content.
///
/// Distinct from [`SkillFile`]/[`WorkflowFile`] because plugin paths are
/// *computed* (`skills/<plugin-name>/SKILL.md`, `workflows/<file>`) rather
/// than fixed literals, so `relative_path` must be an owned `String` rather
/// than the `&'static str` those two types use. Widening `SkillFile` instead
/// would change a public type on the raw emission surface, which plugin mode
/// deliberately does not touch.
pub struct PluginFile {
    /// Path relative to the plugin root (e.g. `"skills/roadmap/SKILL.md"`).
    pub relative_path: String,
    /// The full content of the file.
    pub content: String,
}

/// Returns `true` when `b` may appear *inside* a kebab-case `rdm-` token.
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-'
}

/// Returns `true` when `b` immediately preceding an `rdm-` occurrence means
/// that occurrence is NOT the start of a token (e.g. the `-` in `--rdm-bin`,
/// or the `r` in a hypothetical `librdm-do`).
fn is_token_boundary_blocker(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Rewrites the eleven emitted skill names in `body` to their plugin-mode
/// form, leaving every other `rdm-`-prefixed identifier byte-for-byte alone.
///
/// The scan is token-exact rather than substring-based: an occurrence of
/// `rdm-` only starts a token when the preceding byte is not
/// `[A-Za-z0-9_-]`; the token is then the maximal following run of
/// `[A-Za-z0-9-]` with any trailing `-` trimmed; and it is substituted only
/// when the WHOLE token is a key in [`PLUGIN_SKILL_NAMES`].
///
/// That construction is what makes the rename simultaneously **total** (the
/// frontmatter `name:` line and every cross-reference in prose are part of
/// the same string, so one pass covers them all) and **exact**:
///
/// - `rdm-document` renames to `document`, not to a `rdm-do`-prefixed hybrid,
///   because the token is matched whole;
/// - `rdm-wf-dispatch-phase`, `rdm-wf-estimate`, `rdm-mechanical`,
///   `rdm-next`, `rdm-side` and `rdm-review-on-finalize` are all left verbatim
///   because none is a table key;
/// - `--rdm-bin` is not even considered a token, because `-` precedes it.
fn rewrite_skill_names(body: &str) -> String {
    const NEEDLE: &[u8] = b"rdm-";
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut copied = 0usize;
    let mut i = 0usize;
    while i + NEEDLE.len() <= bytes.len() {
        if &bytes[i..i + NEEDLE.len()] != NEEDLE
            || (i > 0 && is_token_boundary_blocker(bytes[i - 1]))
        {
            i += 1;
            continue;
        }
        let mut end = i + NEEDLE.len();
        while end < bytes.len() && is_token_byte(bytes[end]) {
            end += 1;
        }
        let mut token_end = end;
        while token_end > i && bytes[token_end - 1] == b'-' {
            token_end -= 1;
        }
        // `i` and `token_end` bound a run of ASCII bytes, so both are UTF-8
        // char boundaries and slicing here can never panic.
        match PLUGIN_SKILL_NAMES
            .iter()
            .find(|(raw, _)| *raw == &body[i..token_end])
        {
            Some((_, plugin_name)) => {
                out.push_str(&body[copied..i]);
                out.push_str(plugin_name);
                copied = token_end;
                i = token_end;
            }
            // Not a skill name: skip past the whole token so no shorter
            // lookalike inside it can be matched.
            None => i = end,
        }
    }
    out.push_str(&body[copied..]);
    out
}

/// Rewrites every **bare** mention of `stem` (or of `<stem>.js`) in `body` to
/// the namespaced plugin form `rdm:<stem>`, leaving already-namespaced and
/// path-embedded occurrences alone.
///
/// The scan is token-exact, mirroring [`rewrite_skill_names`]: an occurrence
/// only counts when the preceding byte neither continues an identifier
/// (`[A-Za-z0-9_-]`) nor is a `:` (already namespaced) or `/` (still inside a
/// path literal), and when the byte following the match — after optionally
/// absorbing a `.js` file-name suffix — does not continue an identifier.
/// Absorbing `.js` is what turns a file-name mention such as
/// `` `rdm-wf-dispatch-phase.js` `` into the invocation form
/// `` `rdm:rdm-wf-dispatch-phase` `` rather than a nonsensical
/// `` `rdm:rdm-wf-dispatch-phase.js` ``.
fn namespace_engine_refs(body: &str, stem: &str) -> String {
    let bytes = body.as_bytes();
    let needle = stem.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut copied = 0usize;
    let mut i = 0usize;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] != needle {
            i += 1;
            continue;
        }
        if i > 0 {
            let prev = bytes[i - 1];
            if is_token_boundary_blocker(prev) || prev == b':' || prev == b'/' {
                i += needle.len();
                continue;
            }
        }
        let mut end = i + needle.len();
        let after_js = end + ".js".len();
        if bytes[end..].starts_with(b".js")
            && (after_js >= bytes.len() || !is_token_byte(bytes[after_js]))
        {
            end = after_js;
        }
        // Reject anything that continues the identifier, and anything carrying
        // a file extension other than the `.js` absorbed above (`.json`,
        // `.js.map`, …) — a sentence-ending `.` followed by whitespace is not
        // an extension and does not block the match.
        if end < bytes.len()
            && (is_token_byte(bytes[end])
                || (bytes[end] == b'.'
                    && bytes.get(end + 1).is_some_and(u8::is_ascii_alphanumeric)))
        {
            i += needle.len();
            continue;
        }
        // `i` and `end` bound a run of ASCII bytes, so both are UTF-8 char
        // boundaries and slicing here can never panic.
        out.push_str(&body[copied..i]);
        out.push_str(PLUGIN_NAME);
        out.push(':');
        out.push_str(stem);
        copied = end;
        i = end;
    }
    out.push_str(&body[copied..]);
    out
}

/// Rewrites every reference a skill body makes to a shipped workflow engine
/// into the plugin-mode invocation form, and drops the raw-surface
/// provisioning clause that plugin installs make false.
///
/// Plugin shims invoke engines by namespaced NAME —
/// `` `rdm:rdm-wf-dispatch-phase` `` — rather than by a
/// `${CLAUDE_PLUGIN_ROOT}/workflows/<name>.js` `scriptPath`
/// (`docs/plugin-distribution.md`, Decision 3). Engine names keep their
/// `rdm-wf-` prefix (Decision 2), which is exactly what keeps the emitted
/// skill names and the emitted engine names disjoint.
///
/// The rewrite is **total over every mention**, not just over the source-tree
/// path literals: a plugin-installed shim reaches its engine only through the
/// namespace, so a bare `` `rdm-wf-dispatch-phase` `` left in an operative
/// "invoke the Workflow with …" instruction would not resolve. Both passes run
/// here:
///
/// 1. `.claude/workflows/<file>.js` path literals collapse to `rdm:<stem>`;
/// 2. every remaining bare `<stem>` / `<stem>.js` token is namespaced by
///    [`namespace_engine_refs`], which skips the ones pass 1 already produced.
///
/// Only [`SHIPPED_WORKFLOWS`] stems are namespaced. Engines this
/// distribution does **not** ship — `rdm-wf-estimate`, referenced in prose
/// explaining why the shipped autopilot has no estimate pre-pass — stay bare,
/// because namespacing them would name a plugin entry that does not exist.
///
/// Applied BEFORE [`rewrite_skill_names`]. The two are in fact
/// order-independent — the `rdm:<engine>` this produces has the kebab token
/// `rdm-wf-…`, which is not a [`PLUGIN_SKILL_NAMES`] key, and
/// `rewrite_skill_names` never produces a `.claude/workflows/` literal or an
/// engine stem — but the order is pinned here (and asserted by a test) so it
/// cannot drift into mattering unnoticed.
fn rewrite_workflow_refs(body: &str) -> String {
    let mut out = body.to_string();
    for (file_name, _) in SHIPPED_WORKFLOWS {
        let stem = file_name.strip_suffix(".js").unwrap_or(file_name);
        out = out.replace(
            &format!(".claude/workflows/{file_name}"),
            &format!("{PLUGIN_NAME}:{stem}"),
        );
        out = namespace_engine_refs(&out, stem);
    }
    out.replace(
        ", provisioned automatically by `rdm agent-config claude --skills`",
        ", installed by the `rdm` plugin",
    )
}

/// Appends [`PLUGIN_RDM_BIN_NOTE`] to `body` when it needs one.
///
/// Applied last, after both renames, so its own `--rdm-bin` spelling is
/// carried through verbatim.
fn append_plugin_rdm_bin_note(body: String) -> String {
    if !body.contains("rdmBin") {
        return body;
    }
    let mut out = body;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(PLUGIN_RDM_BIN_NOTE);
    out
}

/// Generates the `.claude-plugin/plugin.json` manifest for the `rdm` plugin.
///
/// The `version` is the rdm crate version. There is deliberately **no**
/// `workflows` key: the `workflows/` directory is convention-discovered by
/// the Claude Code runtime, and the manifest key *replaces* the default
/// location rather than adding to it.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::generate_plugin_manifest;
///
/// let manifest = generate_plugin_manifest();
/// let parsed: serde_json::Value = serde_json::from_str(&manifest).unwrap();
/// assert_eq!(parsed["name"], "rdm");
/// assert!(parsed["workflows"].is_null());
/// ```
pub fn generate_plugin_manifest() -> String {
    let manifest = serde_json::json!({
        "name": PLUGIN_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "description": PLUGIN_DESCRIPTION,
        "author": {
            "name": "Edward Paget",
            "url": env!("CARGO_PKG_REPOSITORY"),
        },
    });
    let mut out = serde_json::to_string_pretty(&manifest)
        .expect("a manifest built from string literals always serializes");
    out.push('\n');
    out
}

/// Generates the plugin-layout skill files: the same eleven skills
/// [`generate_skills`] emits, re-rooted at `skills/<plugin-name>/SKILL.md`
/// and passed through the plugin-mode content transforms.
///
/// The raw surface is called unchanged and every transform runs afterwards on
/// its output, so this function cannot alter what `--skills` emits.
///
/// # Panics
///
/// Panics if [`generate_skills`] emits a skill whose directory name is absent
/// from [`PLUGIN_SKILL_NAMES`]. That is a programmer error meaning the two
/// have drifted — a twelfth skill was added without a plugin-mode name —
/// and panicking is preferable to silently shipping a plugin directory that
/// still carries the `rdm-` prefix.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::{SkillOptions, generate_plugin_skills};
///
/// let skills = generate_plugin_skills(&SkillOptions {
///     project: Some("myproj".to_string()),
///     principles_file: None,
///     mcp: false,
/// });
/// assert_eq!(skills.len(), 11);
/// assert_eq!(skills[0].relative_path, "skills/roadmap/SKILL.md");
/// ```
pub fn generate_plugin_skills(opts: &SkillOptions) -> Vec<PluginFile> {
    generate_skills(opts)
        .into_iter()
        .map(|skill| {
            let raw_name = skill
                .relative_path
                .split('/')
                .next()
                .unwrap_or(skill.relative_path);
            let plugin_name = PLUGIN_SKILL_NAMES
                .iter()
                .find(|(raw, _)| *raw == raw_name)
                .map(|(_, plugin_name)| *plugin_name)
                .unwrap_or_else(|| {
                    panic!(
                        "no PLUGIN_SKILL_NAMES entry for emitted skill {raw_name:?}: \
                         generate_skills and the plugin-name table have drifted"
                    )
                });
            let content = append_plugin_rdm_bin_note(rewrite_skill_names(&rewrite_workflow_refs(
                &skill.content,
            )));
            PluginFile {
                relative_path: format!("skills/{plugin_name}/SKILL.md"),
                content,
            }
        })
        .collect()
}

/// Generates the plugin-layout workflow scripts: exactly the set
/// [`generate_workflows`] returns, re-rooted under `workflows/` at the plugin
/// root (a sibling of `skills/`, never inside `.claude-plugin/`).
///
/// Content passes through **verbatim** — there is deliberately no `meta.name`
/// transform. Engine names keep their `rdm-wf-` prefix in plugin mode
/// (`docs/plugin-distribution.md`, Decision 2), so the runtime renders them as
/// `/rdm:rdm-wf-dispatch-phase` and the emitted plugin bytes equal the raw
/// emitted bytes, which in turn equal this repo's own `.claude/workflows/*.js`.
/// That prefix is also the disambiguator that keeps the emitted skill-name set
/// and the emitted engine-name set disjoint: dropping it as well as `rdm-`
/// would collapse the `dispatch-phase` shim and its engine onto one listing
/// entry.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::generate_plugin_workflows;
///
/// let workflows = generate_plugin_workflows();
/// assert_eq!(workflows.len(), 2);
/// assert_eq!(
///     workflows[0].relative_path,
///     "workflows/rdm-wf-dispatch-phase.js"
/// );
/// ```
pub fn generate_plugin_workflows() -> Vec<PluginFile> {
    generate_workflows()
        .into_iter()
        .map(|workflow| PluginFile {
            relative_path: format!("workflows/{}", workflow.relative_path),
            content: workflow.content.to_string(),
        })
        .collect()
}

/// Generates the complete `rdm` plugin tree: the manifest, then the eleven
/// plugin-layout skills, then the two workflow engines — 14 files, all paths
/// relative to the plugin root.
///
/// # Panics
///
/// Panics under the same skill-table drift condition as
/// [`generate_plugin_skills`].
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::{SkillOptions, generate_plugin_files};
///
/// let files = generate_plugin_files(&SkillOptions {
///     project: None,
///     principles_file: None,
///     mcp: false,
/// });
/// assert_eq!(files.len(), 14);
/// assert_eq!(files[0].relative_path, ".claude-plugin/plugin.json");
/// ```
pub fn generate_plugin_files(opts: &SkillOptions) -> Vec<PluginFile> {
    let mut files = vec![PluginFile {
        relative_path: ".claude-plugin/plugin.json".to_string(),
        content: generate_plugin_manifest(),
    }];
    files.extend(generate_plugin_skills(opts));
    files.extend(generate_plugin_workflows());
    files
}

fn skill_principles_note(path: &str) -> String {
    format!(
        "\n## Principles\n\nRead `{path}` before starting. It contains project conventions that should guide your work."
    )
}

fn render_skill(
    template: &str,
    proj_placeholder: &str,
    proj_value: &str,
    principles_note: Option<&str>,
) -> String {
    let principles = principles_note.unwrap_or("");
    template
        .replace(proj_placeholder, proj_value)
        .replace("{principles}", principles)
}

fn skill_roadmap(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-roadmap/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-roadmap-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_do(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-do/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-do-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_document(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-document/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-document-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_review(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-review/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-review-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_estimate(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-estimate/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-estimate-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_dispatch_phase(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-dispatch-phase/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-dispatch-phase-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_autopilot(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-autopilot/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-autopilot-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_land(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-land/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-land-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_revise(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-revise/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-revise-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_backlog(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-backlog/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-backlog-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_plan_review(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-plan-review/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-plan-review-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

/// Options for generating MCP server configuration.
pub struct McpConfigOptions {
    /// Plan repo root path. When `Some`, the generated config includes `--root <path>`.
    pub root: Option<String>,
}

/// Generates a `.mcp.json` configuration for the rdm MCP server.
///
/// The output is a JSON object with an `mcpServers.rdm` entry that tells
/// MCP-aware clients how to launch the rdm MCP server.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::{McpConfigOptions, generate_mcp_config};
///
/// let json = generate_mcp_config(&McpConfigOptions { root: None });
/// assert!(json.contains("mcpServers"));
/// ```
pub fn generate_mcp_config(opts: &McpConfigOptions) -> String {
    let args: Vec<serde_json::Value> = match &opts.root {
        Some(root) => vec![
            serde_json::Value::String("--root".to_string()),
            serde_json::Value::String(root.clone()),
            serde_json::Value::String("mcp".to_string()),
        ],
        None => vec![serde_json::Value::String("mcp".to_string())],
    };

    let config = serde_json::json!({
        "mcpServers": {
            "rdm": {
                "command": "rdm",
                "args": args
            }
        }
    });

    serde_json::to_string_pretty(&config).expect("JSON serialization cannot fail")
}

/// Returns a quoted project name for use in MCP tool call examples.
fn proj_param_str(project: Option<&str>) -> String {
    match project {
        Some(name) => format!("\"{name}\""),
        None => "\"<PROJECT>\"".to_string(),
    }
}

/// Generates MCP-oriented instruction content referencing MCP tool calls.
fn agent_instructions_mcp(project: Option<&str>, principles_file: Option<&str>) -> String {
    let proj_param = proj_param_str(project);
    let principles = principles_file
        .map(|p| format!("\n\n{}", section_principles(p)))
        .unwrap_or_default();
    include_str!("templates/instructions-mcp.md")
        .replace("{proj_param}", &proj_param)
        .replace("\n{principles}", &principles)
}

// ---------- MCP skill generators ----------

fn mcp_tool_name(tool: &str) -> String {
    format!("mcp__rdm__{tool}")
}

fn skill_roadmap_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-roadmap/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-roadmap-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_roadmap_create", "rdm_roadmap_create"),
                ("t_phase_create", "rdm_phase_create"),
                ("t_roadmap_show", "rdm_roadmap_show"),
                ("t_commit", "rdm_commit"),
            ],
        ),
    }
}

fn skill_do_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-do/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-do-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_list", "rdm_phase_list"),
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_list", "rdm_task_list"),
                ("t_task_show", "rdm_task_show"),
                ("t_task_update", "rdm_task_update"),
                ("t_task_create", "rdm_task_create"),
                ("t_worktree_current", "rdm_worktree_current"),
                ("t_worktree_add", "rdm_worktree_add"),
                ("t_commit", "rdm_commit"),
            ],
        ),
    }
}

fn skill_document_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-document/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-document-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_roadmap_show", "rdm_roadmap_show"),
                ("t_phase_show", "rdm_phase_show"),
            ],
        ),
    }
}

fn skill_review_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-review/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-review-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_show", "rdm_task_show"),
                ("t_task_update", "rdm_task_update"),
                ("t_task_create", "rdm_task_create"),
                ("t_commit", "rdm_commit"),
            ],
        ),
    }
}

fn skill_estimate_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-estimate/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-estimate-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_list", "rdm_phase_list"),
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_roadmap_show", "rdm_roadmap_show"),
            ],
        ),
    }
}

fn skill_dispatch_phase_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-dispatch-phase/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-dispatch-phase-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_show", "rdm_phase_show"),
                // The shim stamps the item in-progress itself and passes
                // `alreadyInProgress: true`, so the workflow can skip its own
                // dedicated stamp subagent.
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_update", "rdm_task_update"),
            ],
        ),
    }
}

fn skill_autopilot_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-autopilot/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-autopilot-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_next", "rdm_next"),
                ("t_phase_update", "rdm_phase_update"),
                // Read-back confirmation after an advance/park write, mirroring
                // the CLI variant's `rdm phase show --format json` call.
                ("t_phase_show", "rdm_phase_show"),
            ],
        ),
    }
}

fn skill_land_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-land/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-land-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_show", "rdm_task_show"),
                ("t_task_update", "rdm_task_update"),
                ("t_worktree_current", "rdm_worktree_current"),
                ("t_worktree_remove", "rdm_worktree_remove"),
            ],
        ),
    }
}

fn skill_revise_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-revise/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-revise-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_review_requests", "rdm_review_requests"),
                ("t_review_show", "rdm_review_show"),
                ("t_review_address_comment", "rdm_review_address_comment"),
                ("t_review_complete", "rdm_review_complete"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_update", "rdm_task_update"),
                ("t_roadmap_update", "rdm_roadmap_update"),
                ("t_commit", "rdm_commit"),
            ],
        ),
    }
}

fn skill_plan_review_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-plan-review/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-plan-review-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_show", "rdm_task_show"),
                ("t_task_update", "rdm_task_update"),
                ("t_task_create", "rdm_task_create"),
                ("t_roadmap_show", "rdm_roadmap_show"),
                ("t_roadmap_update", "rdm_roadmap_update"),
                ("t_commit", "rdm_commit"),
            ],
        ),
    }
}

/// Builds the `rdm-backlog` MCP skill.
///
/// Unlike every other `skill_*_mcp` generator, this template also embeds
/// literal, never-executed `rdm` CLI command text in its "Grooming analysis"
/// section (`rdm task update`, `rdm promote`, `rdm task merge`, `rdm roadmap
/// archive`) — those commands are copy-paste output for a human to run
/// later, not MCP tool calls this skill makes, since no MCP tool exists for
/// merge/archive/promote. That literal CLI text still needs a concrete
/// `--project` flag to be ready to paste, so this is the one MCP skill that
/// also substitutes `{proj_flag}` (computed the same way the CLI skills
/// compute it) alongside the usual `{proj_param}`/`{t_*}` substitutions.
fn skill_backlog_mcp(proj: &str, proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    let rendered = render_mcp_skill(
        include_str!("templates/skill-backlog-mcp.md"),
        proj,
        principles_note,
        &[
            ("t_backlog_report", "rdm_backlog_report"),
            ("t_roadmap_list", "rdm_roadmap_list"),
            ("t_search", "rdm_search"),
        ],
    );
    SkillFile {
        relative_path: "rdm-backlog/SKILL.md",
        content: rendered.replace("{proj_flag}", proj_flag),
    }
}

fn render_mcp_skill(
    template: &str,
    proj: &str,
    principles_note: Option<&str>,
    tools: &[(&str, &str)],
) -> String {
    let principles = principles_note.unwrap_or("");
    let mut result = template
        .replace("{proj_param}", proj)
        .replace("{principles}", principles);
    for (placeholder, tool) in tools {
        result = result.replace(&format!("{{{placeholder}}}"), &mcp_tool_name(tool));
    }
    result
}

fn section_principles(path: &str) -> String {
    format!(
        r#"## Principles

Read `{path}` before starting implementation work. It contains project conventions and design principles that should guide your decisions."#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_from_str_valid() {
        assert_eq!("claude".parse::<Platform>().unwrap(), Platform::Claude);
        assert_eq!("agents-md".parse::<Platform>().unwrap(), Platform::AgentsMd);
        assert_eq!("cursor".parse::<Platform>().unwrap(), Platform::Cursor);
        assert_eq!("copilot".parse::<Platform>().unwrap(), Platform::Copilot);
        assert_eq!("pi".parse::<Platform>().unwrap(), Platform::Pi);
    }

    #[test]
    fn platform_from_str_case_insensitive() {
        assert_eq!("Claude".parse::<Platform>().unwrap(), Platform::Claude);
        assert_eq!("CURSOR".parse::<Platform>().unwrap(), Platform::Cursor);
    }

    #[test]
    fn platform_from_str_invalid() {
        let err = "vim".parse::<Platform>().unwrap_err();
        assert!(err.contains("unknown platform"));
        assert!(err.contains("vim"));
    }

    #[test]
    fn platform_display() {
        assert_eq!(Platform::Claude.to_string(), "claude");
        assert_eq!(Platform::AgentsMd.to_string(), "agents-md");
        assert_eq!(Platform::Cursor.to_string(), "cursor");
        assert_eq!(Platform::Copilot.to_string(), "copilot");
        assert_eq!(Platform::Pi.to_string(), "pi");
    }

    #[test]
    fn conventional_path() {
        assert_eq!(Platform::Claude.conventional_path(), "CLAUDE.md");
        assert_eq!(Platform::AgentsMd.conventional_path(), "AGENTS.md");
        assert_eq!(
            Platform::Cursor.conventional_path(),
            ".cursor/rules/rdm.mdc"
        );
        assert_eq!(
            Platform::Copilot.conventional_path(),
            ".github/copilot-instructions.md"
        );
        assert_eq!(Platform::Pi.conventional_path(), ".pi/AGENTS.md");
    }

    #[test]
    fn pi_user_level_instruction_path() {
        let path = Platform::Pi.user_level_instruction_path().unwrap();
        assert!(
            path.ends_with(".pi/agent/AGENTS.md"),
            "unexpected path: {}",
            path.display()
        );
    }

    #[test]
    fn project_skills_subdir_for_claude_and_pi() {
        assert_eq!(
            Platform::Claude.project_skills_subdir(),
            Some(".claude/skills")
        );
        assert_eq!(Platform::Pi.project_skills_subdir(), Some(".pi/skills"));
        assert_eq!(Platform::AgentsMd.project_skills_subdir(), None);
        assert_eq!(Platform::Cursor.project_skills_subdir(), None);
        assert_eq!(Platform::Copilot.project_skills_subdir(), None);
    }

    #[test]
    fn user_level_skills_dir_for_claude() {
        let path = Platform::Claude.user_level_skills_dir().unwrap();
        assert!(
            path.ends_with(".claude/skills"),
            "unexpected path: {}",
            path.display()
        );
    }

    #[test]
    fn user_level_skills_dir_for_pi() {
        let path = Platform::Pi.user_level_skills_dir().unwrap();
        assert!(
            path.ends_with(".pi/agent/skills"),
            "unexpected path: {}",
            path.display()
        );
    }

    #[test]
    fn user_level_skills_dir_rejects_unsupported() {
        assert!(Platform::Cursor.user_level_skills_dir().is_err());
        assert!(Platform::AgentsMd.user_level_skills_dir().is_err());
        assert!(Platform::Copilot.user_level_skills_dir().is_err());
    }

    #[test]
    fn claude_user_level_instruction_path_default() {
        let path = Platform::Claude.user_level_instruction_path().unwrap();
        assert!(
            path.ends_with(".claude/CLAUDE.md"),
            "unexpected path: {}",
            path.display()
        );
    }

    #[test]
    fn generate_with_project_name() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("--project myproj"));
        assert!(!content.contains("<PROJECT>"));
    }

    #[test]
    fn cli_instructions_teach_document_reviews() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("## Document reviews"));
        assert!(content.contains("rdm review start --on task/<slug>"));
        assert!(content.contains("rdm review submit <review-id> --verdict request-changes"));
        assert!(content.contains("--occurrence <n>"));
        assert!(content.contains("--type review"));
        // Drifted-range semantics are spelled out for agents (JSON ranges
        // index the created_commit body).
        assert!(content.contains("drifted"));
        assert!(content.contains("created_commit"));
        // The project flag is substituted into the review commands too.
        assert!(content.contains("rdm review requests --project myproj"));
        // The agent loop is automated by the rdm-revise skill.
        assert!(content.contains("rdm-revise"));
    }

    #[test]
    fn mcp_instructions_teach_document_reviews() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("## Document reviews"));
        // The four review tools and the loop.
        assert!(content.contains("rdm_review_requests"));
        assert!(content.contains("rdm_review_show"));
        assert!(content.contains("rdm_review_address_comment"));
        assert!(content.contains("rdm_review_complete"));
        assert!(content.contains("rdm-revise"));
        // Anchor/resolution semantics an agent must know.
        assert!(content.contains("anchor_type"));
        assert!(content.contains("drifted"));
        assert!(content.contains("body_at_created_commit"));
        // Commit provenance threading from update responses.
        assert!(content.contains("Commit: <sha>"));
        assert!(content.contains("applied_commit"));
        // The project param is substituted.
        assert!(content.contains("review_id: \"<id>\""));
        assert!(!content.contains("<PROJECT>"));
    }

    #[test]
    fn generate_without_project_name() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("--project <PROJECT>"));
    }

    #[test]
    fn generate_contains_key_sections() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("# rdm"));
        assert!(content.contains("## Setup"));
        assert!(content.contains("## Discovering work"));
        assert!(content.contains("## Reading details"));
        assert!(content.contains("## Updating status"));
        assert!(content.contains("## Creating items"));
        assert!(content.contains("## Body content"));
        assert!(content.contains("## Planning workflow"));
        assert!(content.contains("## Status transitions"));
    }

    #[test]
    fn generate_contains_key_commands() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("rdm roadmap list"));
        assert!(content.contains("rdm task list"));
        assert!(content.contains("rdm roadmap show"));
        assert!(content.contains("rdm phase show"));
        assert!(content.contains("rdm task show"));
        assert!(content.contains("rdm phase update"));
        assert!(content.contains("rdm task update"));
        assert!(content.contains("rdm roadmap create"));
        assert!(content.contains("rdm phase create"));
        assert!(content.contains("rdm task create"));
        assert!(content.contains("--no-edit"));
        assert!(content.contains("--no-body"));
    }

    #[test]
    fn cursor_has_mdc_frontmatter() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::Cursor,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.starts_with("---\n"));
        assert!(content.contains("description:"));
        assert!(content.contains("globs:"));
        // Should still have the instructions after frontmatter
        assert!(content.contains("# rdm"));
    }

    #[test]
    fn claude_no_mdc_frontmatter() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::Claude,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(!content.starts_with("---"));
    }

    #[test]
    fn copilot_no_mdc_frontmatter() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::Copilot,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(!content.starts_with("---"));
    }

    #[test]
    fn planning_workflow_section_contains_key_steps() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("### Before starting work"));
        assert!(content.contains("### Implementing a roadmap phase"));
        assert!(content.contains("### Discovering bugs or side-work"));
        assert!(content.contains("### When a task grows too complex"));
        assert!(content.contains("rdm promote"));
    }

    #[test]
    fn planning_workflow_uses_project_flag() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: false,
        });
        // Workflow section should embed the project flag
        assert!(content.contains("rdm roadmap list --project myproj"));
        assert!(content.contains("rdm task list --project myproj"));
    }

    #[test]
    fn status_transitions_documents_phase_statuses() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("### Phase statuses"));
        assert!(content.contains("`not-started` → `in-progress`"));
        assert!(content.contains("`in-progress` → `done`"));
        assert!(content.contains("`in-progress` → `blocked`"));
        assert!(content.contains("`blocked` → `in-progress`"));
        assert!(content.contains("`in-progress` → `needs-review`"));
        assert!(content.contains("`needs-review` → `reviewed`"));
        assert!(content.contains("`reviewed` → `done`"));
    }

    #[test]
    fn status_transitions_documents_task_statuses() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("### Task statuses"));
        assert!(content.contains("`open` → `in-progress`"));
        assert!(content.contains("`in-progress` → `done`"));
        assert!(content.contains("`in-progress` → `wont-fix`"));
        assert!(content.contains("`open` → `wont-fix`"));
        assert!(content.contains("`in-progress` → `needs-review`"));
        assert!(content.contains("`needs-review` → `reviewed`"));
        assert!(content.contains("`reviewed` → `done`"));
    }

    #[test]
    fn principles_section_included_when_file_specified() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: Some("docs/principles.md".to_string()),
            mcp: false,
        });
        assert!(content.contains("## Principles"));
        assert!(content.contains("docs/principles.md"));
    }

    #[test]
    fn principles_section_excluded_when_no_file() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(!content.contains("## Principles"));
    }

    // --- Skill generation tests ---

    #[test]
    fn generate_skills_returns_eleven_files() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        // Workflows are a separate emission surface (see `generate_workflows`)
        // and are not counted here — this assertion should not grow when the
        // workflow lane gains or loses files.
        assert_eq!(skills.len(), 11);
    }

    #[test]
    fn generate_skills_mcp_returns_eleven_files() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        // Was 10 (no `rdm-backlog` MCP twin) before `skill_backlog_mcp` was
        // added — now matches the non-mcp branch's count.
        assert_eq!(skills.len(), 11);
    }

    #[test]
    fn generate_skills_cli_mcp_name_parity() {
        // The same skill *names* (identified by relative_path) must be
        // emitted on both platforms — a BTreeSet comparison so a future
        // one-sided addition (a skill added to only one branch) fails even
        // if both vecs coincidentally stay the same length or reorder.
        let cli: std::collections::BTreeSet<&str> = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        })
        .iter()
        .map(|s| s.relative_path)
        .collect();
        let mcp: std::collections::BTreeSet<&str> = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        })
        .iter()
        .map(|s| s.relative_path)
        .collect();
        assert_eq!(
            cli, mcp,
            "cli and mcp must emit the same set of skill relative_paths"
        );
    }

    #[test]
    fn generate_skills_correct_paths() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert_eq!(skills[0].relative_path, "rdm-roadmap/SKILL.md");
        assert_eq!(skills[1].relative_path, "rdm-do/SKILL.md");
        assert_eq!(skills[2].relative_path, "rdm-review/SKILL.md");
        assert_eq!(skills[3].relative_path, "rdm-document/SKILL.md");
        assert_eq!(skills[4].relative_path, "rdm-estimate/SKILL.md");
        assert_eq!(skills[5].relative_path, "rdm-dispatch-phase/SKILL.md");
        assert_eq!(skills[6].relative_path, "rdm-autopilot/SKILL.md");
        assert_eq!(skills[7].relative_path, "rdm-land/SKILL.md");
        assert_eq!(skills[8].relative_path, "rdm-revise/SKILL.md");
        assert_eq!(skills[9].relative_path, "rdm-plan-review/SKILL.md");
        assert_eq!(skills[10].relative_path, "rdm-backlog/SKILL.md");
    }

    // --- Workflow generation tests ---

    #[test]
    fn generate_workflows_returns_two_files() {
        let workflows = generate_workflows();
        assert_eq!(workflows.len(), 2);
        assert_eq!(workflows[0].relative_path, "rdm-wf-dispatch-phase.js");
        assert_eq!(workflows[1].relative_path, "rdm-wf-review-refute-fix.js");
    }

    #[test]
    fn generate_workflows_are_byte_identical_to_source() {
        // Intentionally a live comparison against the checked-in
        // `.claude/workflows/*.js` files (not a hardcoded fixture), so a
        // future hand-edit to either the templates or the dogfood copies
        // that isn't mirrored to the other fails CI immediately.
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("rdm-core manifest dir has a parent");
        for workflow in generate_workflows() {
            let source_path = repo_root
                .join(".claude/workflows")
                .join(workflow.relative_path);
            let source = std::fs::read_to_string(&source_path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", source_path.display()));
            assert_eq!(
                workflow.content, source,
                "{} drifted from the embedded template",
                workflow.relative_path
            );
        }
    }

    // --- Agent-definition generation tests ---

    #[test]
    fn generate_agents_returns_one_file() {
        let agents = generate_agents();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].relative_path, "rdm-mechanical.md");
    }

    #[test]
    fn generate_agents_byte_identical_to_source() {
        // Same rationale as `generate_workflows_are_byte_identical_to_source`:
        // a live comparison against the checked-in `.claude/agents/*.md`
        // files, not a hardcoded fixture, so a future hand-edit to either the
        // template or the dogfood copy that isn't mirrored to the other
        // fails CI immediately.
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("rdm-core manifest dir has a parent");
        for agent in generate_agents() {
            let source_path = repo_root.join(".claude/agents").join(agent.relative_path);
            let source = std::fs::read_to_string(&source_path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", source_path.display()));
            assert_eq!(
                agent.content, source,
                "{} drifted from the embedded template",
                agent.relative_path
            );
        }
    }

    // --- Superseded-workflow cleanup tests ---

    /// Leaks a runtime-computed `String` to a `&'static str`, for building
    /// synthetic [`SupersededWorkflow`] tables in tests (the field requires
    /// `'static`, but fingerprints are naturally computed at test time via
    /// [`sha256_hex`]). Test-only; the leak is bounded by the test process.
    fn leak_str(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }

    /// Leaks a runtime-built `Vec<T>` to a `&'static [T]`, for the same
    /// reason as [`leak_str`]: [`SupersededWorkflow::fingerprints`] requires
    /// `'static`, but test tables are naturally built at test time.
    fn leak_slice<T>(v: Vec<T>) -> &'static [T] {
        Box::leak(v.into_boxed_slice())
    }

    // The production table is no longer empty: the `rdm-wf-` engine rename
    // populated it. These assertions pin the entries deliberately rather than
    // reading them back off the table, so a silent edit fails here.
    #[test]
    fn superseded_workflows_table_names_the_renamed_and_retired_engines() {
        let names: Vec<&str> = SUPERSEDED_WORKFLOWS.iter().map(|e| e.name).collect();
        assert_eq!(
            names,
            vec!["dispatch-phase.js", "review-refute-fix.js", "autopilot.js"],
            "the production table must carry exactly the two renamed engines \
             plus the retired autopilot orphan"
        );

        let successors: Vec<Option<&str>> =
            SUPERSEDED_WORKFLOWS.iter().map(|e| e.successor).collect();
        assert_eq!(
            successors,
            vec![
                Some("rdm-wf-dispatch-phase.js"),
                Some("rdm-wf-review-refute-fix.js"),
                None,
            ],
            "autopilot.js is retired outright and must carry no successor"
        );

        // Every entry must actually be able to match something: an empty
        // fingerprint list is a structural no-op that would silently never
        // clean up.
        for entry in SUPERSEDED_WORKFLOWS {
            assert!(
                !entry.fingerprints.is_empty(),
                "{} must carry at least one fingerprint",
                entry.name
            );
        }
    }

    #[test]
    fn superseded_workflow_names_never_collide_with_shipped_names() {
        for entry in SUPERSEDED_WORKFLOWS {
            assert!(
                !SHIPPED_WORKFLOWS
                    .iter()
                    .any(|(path, _)| *path == entry.name),
                "{} is both shipped and superseded — cleanup would delete a \
                 file the same emission just wrote",
                entry.name
            );
        }
    }

    #[test]
    fn superseded_fingerprints_are_well_formed_and_never_match_a_shipped_body() {
        // A shipped body must never fingerprint into the superseded table:
        // that would make an emission delete a file the same emission just
        // wrote. (The names already can't collide — see the test above — but
        // this catches a copy-pasted digest, which the name check cannot.)
        let shipped: Vec<String> = generate_workflows()
            .iter()
            .map(|w| sha256_hex(w.content.as_bytes()))
            .collect();
        for entry in SUPERSEDED_WORKFLOWS {
            for fp in entry.fingerprints {
                assert_eq!(
                    fp.len(),
                    64,
                    "{}: {fp} is not a SHA-256 hex digest",
                    entry.name
                );
                assert!(
                    fp.chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                    "{}: {fp} must be lowercase hex",
                    entry.name
                );
                assert!(
                    !shipped.contains(&fp.to_string()),
                    "{}: fingerprint {fp} matches a CURRENTLY-shipped workflow body — \
                     cleanup would delete a file this same emission wrote",
                    entry.name
                );
            }
        }
    }

    #[test]
    fn resolve_superseded_workflows_empty_table_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.js");
        std::fs::write(&a, b"alpha").unwrap();
        std::fs::write(&b, b"bravo").unwrap();

        let outcomes = resolve_superseded_workflows(dir.path(), &[]);

        assert!(outcomes.is_empty());
        assert_eq!(std::fs::read(&a).unwrap(), b"alpha");
        assert_eq!(std::fs::read(&b).unwrap(), b"bravo");
    }

    #[test]
    fn resolve_superseded_workflows_shipped_table_ignores_unnamed_files() {
        // The shipped table is no longer empty, so this replaces the old
        // "empty table is a structural no-op" guarantee: a directory holding
        // no file the table names must still come back untouched.
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("custom-local.js");
        std::fs::write(&a, b"alpha").unwrap();

        let outcomes = resolve_superseded_workflows(dir.path(), SUPERSEDED_WORKFLOWS);

        assert!(outcomes.is_empty());
        assert_eq!(std::fs::read(&a).unwrap(), b"alpha");
    }

    #[test]
    fn resolve_superseded_workflows_removes_fingerprint_match() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"old-workflow-body-for-fingerprint-match-test\n";
        let path = dir.path().join("old-name.js");
        std::fs::write(&path, content).unwrap();
        let digest = leak_str(sha256_hex(content));
        let fingerprints = leak_slice(vec![digest]);

        let table = [SupersededWorkflow {
            name: "old-name.js",
            fingerprints,
            successor: Some("new-name.js"),
        }];
        let outcomes = resolve_superseded_workflows(dir.path(), &table);

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            SupersededOutcome::Removed { path: p } => assert_eq!(p, &path),
            other => panic!("expected Removed, got {other:?}"),
        }
        // Self-test / removal-branch proof: this is a disk-state assertion,
        // not merely a check on the returned enum. Commenting out the
        // `std::fs::remove_file` call inside `resolve_superseded_workflows`'s
        // fingerprint-match branch would leave this file on disk and turn
        // this assertion red even if a stub still returned `Removed`.
        assert!(!path.exists(), "file should have been removed from disk");
    }

    #[test]
    fn resolve_superseded_workflows_skips_modified_content() {
        let dir = tempfile::tempdir().unwrap();
        let original = b"old-workflow-body-that-user-has-since-modified\n";
        let path = dir.path().join("old-name.js");
        std::fs::write(&path, original).unwrap();
        // The table's only known fingerprint is for different content, so
        // the on-disk file (as modified by the user) matches nothing.
        let unrelated_digest = leak_str(sha256_hex(b"a completely different original body\n"));
        let fingerprints = leak_slice(vec![unrelated_digest]);

        let table = [SupersededWorkflow {
            name: "old-name.js",
            fingerprints,
            successor: Some("new-name.js"),
        }];
        let outcomes = resolve_superseded_workflows(dir.path(), &table);

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            SupersededOutcome::SkippedModified { path: p } => assert_eq!(p, &path),
            other => panic!("expected SkippedModified, got {other:?}"),
        }
        // Disk-state proof: if the skip branch were replaced by an
        // unconditional match, this file would be missing instead.
        assert_eq!(
            std::fs::read(&path).unwrap(),
            original,
            "modified file must be left byte-for-byte untouched"
        );
    }

    #[test]
    fn resolve_superseded_workflows_skips_empty_fingerprint_list() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"some content";
        let path = dir.path().join("old-name.js");
        std::fs::write(&path, content).unwrap();

        let table = [SupersededWorkflow {
            name: "old-name.js",
            fingerprints: &[],
            successor: None,
        }];
        let outcomes = resolve_superseded_workflows(dir.path(), &table);

        assert_eq!(outcomes.len(), 1);
        assert!(matches!(
            outcomes[0],
            SupersededOutcome::SkippedModified { .. }
        ));
        assert_eq!(std::fs::read(&path).unwrap(), content);
    }

    #[test]
    fn resolve_superseded_workflows_skips_directory_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("old-name.js");
        std::fs::create_dir(&subdir).unwrap();

        let table = [SupersededWorkflow {
            name: "old-name.js",
            fingerprints: &["deadbeef"],
            successor: None,
        }];
        let outcomes = resolve_superseded_workflows(dir.path(), &table);

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            SupersededOutcome::SkippedModified { path: p } => assert_eq!(p, &subdir),
            other => panic!("expected SkippedModified, got {other:?}"),
        }
        assert!(subdir.is_dir(), "directory must survive untouched");
    }

    #[test]
    fn resolve_superseded_workflows_ignores_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        let unrelated = dir.path().join("notes.txt");
        std::fs::write(&unrelated, b"my own notes, not an rdm file").unwrap();

        // A synthetic table that names a completely different file — the
        // unrelated file's name never appears in it.
        let table = [SupersededWorkflow {
            name: "old-name.js",
            fingerprints: &["deadbeef"],
            successor: Some("new-name.js"),
        }];
        let outcomes = resolve_superseded_workflows(dir.path(), &table);

        assert!(
            outcomes.iter().all(|o| match o {
                SupersededOutcome::Removed { path }
                | SupersededOutcome::SkippedModified { path }
                | SupersededOutcome::Failed { path, .. } => path != &unrelated,
                SupersededOutcome::InvalidName { .. } => true,
            }),
            "no outcome may reference a file the table never named"
        );
        assert_eq!(
            std::fs::read(&unrelated).unwrap(),
            b"my own notes, not an rdm file"
        );
    }

    #[test]
    fn resolve_superseded_workflows_rejects_path_traversal_names() {
        let root = tempfile::tempdir().unwrap();
        let workflows_dir = root.path().join("workflows");
        std::fs::create_dir(&workflows_dir).unwrap();

        // A file living outside workflows_dir that a `..`-escaping name
        // would resolve to, if traversal were permitted.
        let escape_target = root.path().join("escape.js");
        std::fs::write(&escape_target, b"do not touch me").unwrap();
        let digest = leak_str(sha256_hex(b"do not touch me"));
        let fingerprints = leak_slice(vec![digest]);

        let table = [
            SupersededWorkflow {
                name: "../escape.js",
                fingerprints,
                successor: None,
            },
            SupersededWorkflow {
                name: "sub/escape.js",
                fingerprints,
                successor: None,
            },
        ];
        let outcomes = resolve_superseded_workflows(&workflows_dir, &table);

        // Both entries are reported — as `InvalidName`, not silently
        // dropped — but neither ever resolves a filesystem path, so
        // `escape_target` is untouched regardless.
        assert_eq!(
            outcomes,
            vec![
                SupersededOutcome::InvalidName {
                    name: "../escape.js"
                },
                SupersededOutcome::InvalidName {
                    name: "sub/escape.js"
                },
            ],
            "traversal-shaped names must be rejected before any filesystem match, and reported as InvalidName, got {outcomes:?}"
        );
        assert!(
            escape_target.exists(),
            "file outside workflows_dir must survive"
        );
        assert_eq!(std::fs::read(&escape_target).unwrap(), b"do not touch me");
    }

    #[test]
    #[cfg(unix)]
    fn resolve_superseded_workflows_reports_failed_removal_without_erroring() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let workflows_dir = root.path().join("workflows");
        std::fs::create_dir(&workflows_dir).unwrap();

        let content = b"file-inside-readonly-dir-for-failed-removal-test\n";
        let path = workflows_dir.join("old-name.js");
        std::fs::write(&path, content).unwrap();
        let digest = leak_str(sha256_hex(content));

        // Removing a file requires write+execute permission on its parent
        // directory, not on the file itself — so make workflows_dir
        // read-only-and-traversable to force the unlink to fail with
        // EACCES/EPERM regardless of the file's own permissions.
        std::fs::set_permissions(&workflows_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let fingerprints = leak_slice(vec![digest]);
        let table = [SupersededWorkflow {
            name: "old-name.js",
            fingerprints,
            successor: Some("new-name.js"),
        }];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            resolve_superseded_workflows(&workflows_dir, &table)
        }));

        // Restore permissions unconditionally so the tempdir can be cleaned
        // up regardless of whether the assertions below pass or fail.
        std::fs::set_permissions(&workflows_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        let outcomes = result.expect("resolve_superseded_workflows must not panic");
        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            SupersededOutcome::Failed { path: p, error } => {
                assert_eq!(p, &path);
                assert!(
                    !error.is_empty(),
                    "Failed outcome must carry a non-empty error"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(
            path.exists(),
            "file must still exist after a failed removal"
        );
    }

    #[test]
    fn skill_backlog_documents_the_grooming_plan() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[10].content;
        assert!(content.contains("name: rdm-backlog"));
        assert!(content.contains("$ARGUMENTS"));
        // Read-and-propose: runs backlog report, mutates nothing.
        assert!(content.contains("rdm backlog report --format json"));
        assert!(content.contains("Non-mutation guarantee"));
        assert!(content.contains("never runs"));
        // Each category maps to a literal proposed command.
        assert!(content.contains("task update <slug> --status wont-fix"));
        assert!(content.contains("task merge <survivor> --from"));
        assert!(content.contains("promote <slug> --into <roadmap>"));
        assert!(content.contains("roadmap archive <roadmap>"));
        // Never force-archive; the candidates never need it.
        assert!(content.contains("Never** add `--force`"));
        // Ambiguity degrades to open questions, not blind actions.
        assert!(content.contains("## Open questions"));
        assert!(content.contains("file it as an open question instead"));
        // Empty case is handled explicitly.
        assert!(content.contains("Nothing to groom"));
        // Autopilot-ready framing: proposed phase bodies carry the standard headings.
        assert!(content.contains("## Context` / `## Steps` / `## Acceptance Criteria"));
        assert!(content.contains("rdm-autopilot"));
    }

    #[test]
    fn skill_backlog_mcp_documents_the_grooming_plan() {
        let skills = generate_skills(&SkillOptions {
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: true,
        });
        let content = &skills[10].content;
        assert!(content.contains("name: rdm-backlog"));
        assert!(content.contains("$ARGUMENTS"));
        // The one executed read call is the mcp tool, substituted.
        assert!(content.contains("rdm_backlog_report"));
        assert!(content.contains("rdm_roadmap_list"));
        assert!(content.contains("rdm_search"));
        // No leftover template placeholders.
        assert!(!content.contains("{t_backlog_report}"));
        assert!(!content.contains("{t_roadmap_list}"));
        assert!(!content.contains("{t_search}"));
        assert!(!content.contains("{proj_param}"));
        assert!(!content.contains("{proj_flag}"));
        // allowed-tools omits Bash — mcp skills never shell out.
        assert!(!content.contains("- Bash"));
        assert!(content.contains("Non-mutation guarantee"));
        assert!(content.contains("never calls"));
        // Proposed (never-executed) mutating commands stay literal CLI text,
        // now with the concrete project substituted in.
        assert!(content.contains("task update <slug> --status wont-fix"));
        assert!(content.contains("task merge <survivor> --from"));
        assert!(content.contains("promote <slug> --into <roadmap>"));
        assert!(content.contains("roadmap archive <roadmap>"));
        assert!(content.contains("--project myproj"));
        assert!(content.contains("not MCP tool calls this skill makes"));
        assert!(content.contains("## Open questions"));
        assert!(content.contains("Nothing to groom"));
    }

    #[test]
    fn skill_revise_documents_the_revision_loop() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[8].content;
        assert!(content.contains("name: rdm-revise"));
        assert!(content.contains("$ARGUMENTS"));
        // Distinct from rdm-review: acts on *document* reviews.
        assert!(content.contains("rdm-review"));
        assert!(content.contains("document"));
        // The queue → read → dispatch → apply → record → close loop.
        assert!(content.contains("rdm review requests --format json"));
        assert!(content.contains("rdm review show <review-id> --format json"));
        // Summary-first: comments are instances of the reviewer's intent.
        assert!(content.contains("summary"));
        assert!(content.contains("overall intent"));
        // Anchor dispatch: resolved / drifted / whole-document (incl. Unknown).
        assert!(content.contains("anchor_type"));
        assert!(content.contains("drifted"));
        assert!(content.contains("whole-document"));
        assert!(content.contains("unrecognized `anchor_type`"));
        assert!(content.contains("created_commit"));
        // Edits go through rdm update commands, never direct file edits.
        assert!(content.contains("rdm phase update"));
        assert!(content.contains("rdm task update"));
        assert!(content.contains("rdm roadmap update"));
        // SHA capture immediately after the edit, threaded explicitly.
        assert!(content.contains("rev-parse HEAD"));
        assert!(content.contains("--applied-commit <sha>"));
        assert!(content.contains("before any review update"));
        // Clarification leaves the comment open; wont-fix carries reasoning.
        assert!(content.contains("leave the comment **open** (no `--status`)"));
        assert!(content.contains("--status wont-fix"));
        // Close only when nothing is open; otherwise stay submitted.
        assert!(content.contains("--state addressed"));
        assert!(content.contains("leave the review submitted"));
    }

    #[test]
    fn mcp_skill_revise_uses_review_tools_and_commit_threading() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[8].content;
        assert!(content.contains("name: rdm-revise"));
        assert!(content.contains("$ARGUMENTS"));
        // Drives the loop through the four review tools plus the doc updates.
        assert!(content.contains("rdm_review_requests"));
        assert!(content.contains("rdm_review_show"));
        assert!(content.contains("rdm_review_address_comment"));
        assert!(content.contains("rdm_review_complete"));
        assert!(content.contains("rdm_phase_update"));
        assert!(content.contains("rdm_task_update"));
        assert!(content.contains("rdm_roadmap_update"));
        // Commit provenance is threaded from the update tool's response —
        // the defaulting fallback is best-effort and never fires on wont-fix.
        assert!(content.contains("Commit: <sha>"));
        assert!(content.contains("applied_commit"));
        assert!(content.contains("best-effort"));
        assert!(content.contains("never defaults for `wont-fix`"));
        // Clarification: no status leaves the comment open; complete refuses
        // and lists open ids while clarification is pending.
        assert!(content.contains("no `status`"));
        assert!(content.contains("leave the review submitted"));
        // Anchor dispatch against the inlined document bodies.
        assert!(content.contains("body_at_created_commit"));
        assert!(content.contains("current_body"));
        assert!(content.contains("anchor_type"));
        // MCP variant: Bash-free frontmatter, resolved mcp__rdm__ tool names.
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(!frontmatter.contains("  - Bash"));
        assert!(frontmatter.contains("mcp__rdm__rdm_review_requests"));
        assert!(frontmatter.contains("mcp__rdm__rdm_review_address_comment"));
        assert!(frontmatter.contains("mcp__rdm__rdm_task_update"));
    }

    // --- rdm-plan-review skill tests ---

    #[test]
    fn skill_plan_review_has_correct_name() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[9].content.contains("name: rdm-plan-review"));
    }

    #[test]
    fn skill_plan_review_has_agent_tool() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[9].content.contains("Agent"));
    }

    #[test]
    fn skill_plan_review_contains_arguments_variable() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[9].content.contains("$ARGUMENTS"));
    }

    #[test]
    fn mcp_skill_plan_review_has_correct_name() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(skills[9].content.contains("name: rdm-plan-review"));
    }

    #[test]
    fn mcp_skill_plan_review_contains_arguments_variable() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(skills[9].content.contains("$ARGUMENTS"));
    }

    #[test]
    fn skill_plan_review_covers_three_dimensions() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("**coherence** — *always.*"));
        assert!(content.contains("**architectural-fit** — *always.*"));
        // The unit-of-work dimension is triggered by the target type, not by
        // diff shape: it runs only for a phase.
        assert!(content.contains("**unit-of-work** — *trigger: the target is a phase.*"));
        assert!(
            content.contains("add `unit-of-work` only when the target type from step 1 is a phase")
        );
    }

    #[test]
    fn mcp_skill_plan_review_covers_three_dimensions() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("**coherence** — *always.*"));
        assert!(content.contains("**architectural-fit** — *always.*"));
        assert!(content.contains("**unit-of-work** — *trigger: the target is a phase.*"));
        assert!(
            content.contains("add `unit-of-work` only when the target type from step 1 is a phase")
        );
    }

    #[test]
    fn skill_plan_review_architecture_reviewer_falls_back_to_claude_md() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("CLAUDE.md"));
        assert!(content.contains("AGENTS.md"));
    }

    #[test]
    fn skill_plan_review_dispatches_read_only_reviewers() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("read-only"));
        assert!(content.contains("parallel"));
    }

    #[test]
    fn mcp_skill_plan_review_dispatches_read_only_reviewers() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("read-only"));
        assert!(content.contains("parallel"));
    }

    #[test]
    fn skill_plan_review_consolidates_single_verdict() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("**reviewed**"));
        assert!(content.contains("**rework**"));
        assert!(content.contains("**escalated**"));
        assert!(content.contains("the first matching rule wins"));
        // The retired vocabulary survives only as the explicit mapping note.
        assert!(!content.contains("**PASS WITH CONCERNS**"));
        assert!(!content.contains("**REWORK**"));
    }

    #[test]
    fn mcp_skill_plan_review_consolidates_single_verdict() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("**reviewed**"));
        assert!(content.contains("**rework**"));
        assert!(content.contains("**escalated**"));
        assert!(content.contains("the first matching rule wins"));
        assert!(!content.contains("**PASS WITH CONCERNS**"));
        assert!(!content.contains("**REWORK**"));
    }

    #[test]
    fn skill_plan_review_categorizes_findings() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("Small"));
        assert!(content.contains("Large"));
        // Large findings are filed as tasks; small findings are applied via
        // the plan document's own update command.
        assert!(content.contains("rdm task create"));
        assert!(content.contains("rdm phase update"));
        assert!(content.contains("--body"));
    }

    #[test]
    fn skill_plan_review_orchestrator_applies_fixes_not_subagents() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("never edits"));
        assert!(content.contains("orchestrator"));
        assert!(content.contains("only the orchestrator edits"));
    }

    /// Suggested default tags paired with the leading fragment of each gloss.
    /// Bare tag names would match vacuously (`bug` already occurs ~10x per
    /// template), so assertions anchor to tag+gloss pairs.
    const DEFAULT_TAG_GLOSSES: &[&str] = &[
        "`bug` (defect",
        "`enhancement` (new capability",
        "`cli` (rdm-cli",
        "`core` (rdm-core",
        "`server` (HTTP/MCP",
        "`web-ui` (browser",
        "`docs` (documentation",
    ];

    #[test]
    fn cli_instructions_instruct_tagging_on_create() {
        let content = agent_instructions(None, None);
        assert!(content.contains("**Always pass `--tags` when you create**"));
        assert!(content.contains("untagged items are invisible to tag-filtered queries"));
        assert!(content.contains("replaces the existing list"));
    }

    #[test]
    fn mcp_instructions_instruct_tagging_on_create() {
        let content = agent_instructions_mcp(None, None);
        assert!(content.contains("**Always pass `tags` when you create**"));
        assert!(content.contains("untagged items are invisible to tag-filtered queries"));
        assert!(content.contains("replaces the existing list"));
    }

    #[test]
    fn instructions_suggest_default_tag_vocabulary() {
        let cli = agent_instructions(None, None);
        let mcp = agent_instructions_mcp(None, None);
        for pair in DEFAULT_TAG_GLOSSES {
            assert!(cli.contains(pair), "CLI instructions missing gloss {pair}");
            assert!(mcp.contains(pair), "MCP instructions missing gloss {pair}");
        }
        assert!(cli.contains("not a closed set"));
        assert!(mcp.contains("not a closed set"));
        assert!(cli.contains("--tags bug,cli"));
        assert!(mcp.contains("tags: [\"bug\", \"cli\"]"));
    }

    #[test]
    fn cli_instructions_teach_tag_list_discovery() {
        let content = agent_instructions(None, None);
        assert!(content.contains("rdm tag list --project <PROJECT>"));
        assert!(
            content.contains("Tags are compared verbatim — `CLI` and `cli` are different tags.")
        );
        // Superseded discovery phrasing is gone.
        assert!(!content.contains("rdm search \"\" --tag <candidate>"));

        let scoped = agent_instructions(Some("myproj"), None);
        assert!(scoped.contains("rdm tag list --project myproj"));
    }

    #[test]
    fn mcp_instructions_teach_tag_list_discovery() {
        let content = agent_instructions_mcp(None, None);
        assert!(content.contains("rdm tag list"));
        // No such MCP tool exists — don't invent one.
        assert!(!content.contains("rdm_tag_list"));
    }

    #[test]
    fn instructions_leave_no_unsubstituted_placeholders() {
        for content in [
            agent_instructions(None, None),
            agent_instructions(Some("myproj"), None),
        ] {
            assert!(!content.contains("{proj_flag}"));
            assert!(!content.contains("{proj_param}"));
        }
        for content in [
            agent_instructions_mcp(None, None),
            agent_instructions_mcp(Some("myproj"), None),
        ] {
            // `agent_instructions_mcp` only substitutes `{proj_param}`; a
            // `{proj_flag}` in the MCP template would leak verbatim.
            assert!(!content.contains("{proj_flag}"));
            assert!(!content.contains("{proj_param}"));
        }
    }

    #[test]
    fn skill_plan_review_clears_tag_on_pass() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("needs-plan-review"));
        assert!(content.contains("--format json"));
        assert!(content.contains("--tags"));
        assert!(content.contains(
            "On **reviewed** — when the plan is clean or only has concerns/suggestions:"
        ));
        assert!(content.contains("Read the target's current tags"));
        assert!(content.contains("| **reviewed** | cleared | none |"));
        assert!(
            content.contains("rdm commit -m \"chore(plan): clear needs-plan-review on <target>\"")
        );
    }

    #[test]
    fn mcp_skill_plan_review_clears_tag_on_pass() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("needs-plan-review"));
        assert!(content.contains(
            "On **reviewed** — when the plan is clean or only has concerns/suggestions:"
        ));
        assert!(content.contains("Read the target's current tags via"));
        assert!(content.contains("| **reviewed** | cleared | none |"));
    }

    #[test]
    fn mcp_skill_plan_review_uses_array_tags_not_string() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        // The MCP tags parameter is a JSON array (Option<Vec<String>>), never
        // the CLI's comma-joined string convention.
        assert!(content.contains("tags: [\"<remaining-tag-1>\", \"<remaining-tag-2>\"]"));
        assert!(content.contains("tags: []"));
        assert!(!content.contains("tags: \"<comma-joined-remaining-tags>\""));
        assert!(!content.contains("tags: \"\""));
    }

    #[test]
    fn skill_plan_review_implementation_plan_mode_skips_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("--implementation-plan"));
        assert!(content.contains("no tag-gate step"));
        assert!(content.contains("skip the Gate step entirely for this mode"));
        assert!(content.contains("Skip this step entirely in `--implementation-plan` mode"));
        // The carve-out also survives in the generated gate spec, so it cannot
        // be lost on regeneration.
        assert!(content.contains("**`--implementation-plan`** — **no gate at all.**"));
    }

    #[test]
    fn mcp_skill_plan_review_implementation_plan_mode_skips_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("--implementation-plan"));
        assert!(content.contains("no tag-gate step"));
        assert!(content.contains("skip the Gate step entirely for this mode"));
        assert!(content.contains("Skip this step entirely in `--implementation-plan` mode"));
        // The carve-out also survives in the generated gate spec, so it cannot
        // be lost on regeneration.
        assert!(content.contains("**`--implementation-plan`** — **no gate at all.**"));
    }

    #[test]
    fn skill_plan_review_implementation_plan_mode_skips_step4_mutations() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("the *act* half is skipped entirely"));
        assert!(content.contains("folding them back into the plan text is left to the caller"));
        assert!(content.contains("skips the Act step's fix-application half the same way"));
        assert!(content.contains(
            "Skip this step's fix-application half entirely in `--implementation-plan` mode"
        ));
    }

    #[test]
    fn mcp_skill_plan_review_implementation_plan_mode_skips_step4_mutations() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("the *act* half is skipped entirely"));
        assert!(content.contains("folding them back into the plan text is left to the caller"));
        assert!(content.contains("skips the Act step's fix-application half the same way"));
        assert!(content.contains(
            "Skip this step's fix-application half entirely in `--implementation-plan` mode"
        ));
    }

    #[test]
    fn skill_plan_review_unit_of_work_reviewer_skips_implementation_plan() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        // The phases-only trigger and its skip list are rendered from the
        // canonical source, so assert the (line-wrapped) rendered fragments.
        assert!(content.contains("*trigger: the target is a phase.* Skipped for"));
        assert!(content.contains("tasks, standalone roadmap bodies, and `--implementation-plan`"));
    }

    #[test]
    fn mcp_skill_plan_review_unit_of_work_reviewer_skips_implementation_plan() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        // The phases-only trigger and its skip list are rendered from the
        // canonical source, so assert the (line-wrapped) rendered fragments.
        assert!(content.contains("*trigger: the target is a phase.* Skipped for"));
        assert!(content.contains("tasks, standalone roadmap bodies, and `--implementation-plan`"));
    }

    #[test]
    fn skill_plan_review_leaves_tag_on_rework() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("On **rework** or **escalated** — when changes are needed:"));
        assert!(content.contains(
            "Do **not** call `update --tags`. The `needs-plan-review` tag is left unchanged in place."
        ));
        // Backed by the generated gate spec so it survives regeneration.
        assert!(content.contains("| **rework** | left in place | none |"));
        assert!(content.contains("| **escalated** | left in place | none |"));
    }

    #[test]
    fn mcp_skill_plan_review_leaves_tag_on_rework() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("On **rework** or **escalated** — when changes are needed:"));
        assert!(content.contains(
            "Do **not** call the update tool with `tags`. The `needs-plan-review` tag is left unchanged in place."
        ));
        assert!(content.contains("| **rework** | left in place | none |"));
        assert!(content.contains("| **escalated** | left in place | none |"));
    }

    #[test]
    fn skill_plan_review_explains_tags_replace_semantics() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("`--tags` replaces the whole list"));
    }

    #[test]
    fn skill_plan_review_gates_each_phase_individually_under_roadmap() {
        for mcp in [false, true] {
            let skills = generate_skills(&SkillOptions {
                project: None,
                principles_file: None,
                mcp,
            });
            let content = &skills[9].content;
            assert!(
                content.contains("Under `--roadmap <slug>`, gate each phase **individually**"),
                "mcp={mcp}: per-phase roadmap gating missing from the hand-authored step"
            );
            // Backed by the generated gate spec so it survives regeneration.
            assert!(
                content.contains("gate each phase **individually**, and the roadmap"),
                "mcp={mcp}: per-phase roadmap gating missing from the generated gate spec"
            );
        }
    }

    #[test]
    fn skill_plan_review_spec_confined_to_generated_block() {
        const BEGIN: &str = "<!-- rdm:review-spec:begin";
        const END: &str = "<!-- rdm:review-spec:end -->";
        // Definitional phrases lifted verbatim from the generated plan block.
        // Bare words like "rework" recur legitimately in the hand-authored
        // CLI/MCP mechanics, so only full definitions are listed.
        const DEFINITIONAL_PHRASES: &[&str] = &[
            "the work must not advance as-is",
            "recorded but non-gating",
            "minor optional improvement",
            "Scale the fleet to what the change actually touches",
            "Determine the outcome in this strict order",
            "needs a *human decision*",
            "**Plan-stage severity calibration.**",
            "*trigger: the target is a phase.*",
        ];

        for mcp in [false, true] {
            let skills = generate_skills(&SkillOptions {
                project: None,
                principles_file: None,
                mcp,
            });
            let content = &skills[9].content;
            let begin = content
                .find(BEGIN)
                .unwrap_or_else(|| panic!("mcp={mcp}: missing begin marker"));
            let end = content
                .find(END)
                .unwrap_or_else(|| panic!("mcp={mcp}: missing end marker"));
            assert!(
                begin < end,
                "mcp={mcp}: begin marker must precede end marker"
            );

            let before = &content[..begin];
            let after = &content[end + END.len()..];

            for phrase in DEFINITIONAL_PHRASES {
                assert!(
                    !before.contains(phrase),
                    "mcp={mcp}: spec-definition phrase found before the generated block: {phrase:?}"
                );
                assert!(
                    !after.contains(phrase),
                    "mcp={mcp}: spec-definition phrase found after the generated block: {phrase:?}"
                );
            }
        }
    }

    #[test]
    fn skill_land_documents_landing_and_safety() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[7].content;
        assert!(content.contains("name: rdm-land"));
        // Item ref comes from $ARGUMENTS.
        assert!(content.contains("$ARGUMENTS"));
        assert!(content.contains("item ref"));
        // Linear history via rebase + fast-forward merge, no merge commit.
        assert!(content.contains("merge --ff-only"));
        assert!(content.contains("linear history"));
        assert!(content.contains("rebase"));
        assert!(content.contains("no merge commit"));
        // Preconditions: reviewed, the Done: line, the CI-equivalent checks.
        assert!(content.contains("reviewed"));
        assert!(content.contains("Done:"));
        // CI-equivalent checks are discovered from the consuming repo, not hardcoded to rdm's
        // own Rust toolchain (rdm's cargo triad appears only as an illustrative parenthetical).
        assert!(content.contains("CI config"));
        assert!(content.contains("docs/principles.md"));
        assert!(content.contains("CLAUDE.md"));
        assert!(content.contains("AGENTS.md"));
        assert!(content.contains("abort and escalate"));
        assert!(content.contains("no CI-equivalent checks determinable"));
        // Abort-and-escalate on conflict/failure per the shared protocol; never force.
        assert!(content.contains("git rebase --abort"));
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("never force"));
        // Post-commit flips reviewed -> done; idempotent fallback exists.
        assert!(content.contains("reviewed → done"));
        assert!(content.contains("post-commit"));
        assert!(content.contains("--status done --commit"));
        // Cleanup: single remove + batch prune.
        assert!(content.contains("rdm worktree remove <item> --delete-branch"));
        assert!(content.contains("rdm worktree prune"));
        // Safety posture: explicit/opt-in only, never auto-lands.
        assert!(content.contains("never auto-lands"));
        assert!(content.contains("--land"));
    }

    #[test]
    fn skill_land_synthesizes_the_completion_trailer_before_the_rebase() {
        for mcp in [false, true] {
            let skills = generate_skills(&SkillOptions {
                project: None,
                principles_file: None,
                mcp,
            });
            let content = &skills[7].content;
            // Precondition 2 reads the completion policy off the autonomous
            // OUTCOME rather than inferring it from a missing trailer...
            assert!(
                content.contains("`writesCompletion: true` on `reviewed`"),
                "skill-land (mcp={mcp}) must state the OUTCOME carries writesCompletion on reviewed"
            );
            assert!(
                content.contains("Read the policy off the outcome, do not infer it"),
                "skill-land (mcp={mcp}) must instruct the lander to read the policy, not infer it"
            );
            // ...synthesizes the line from rdm (one home for the format)...
            assert!(
                content.contains("rdm hook done-line"),
                "skill-land (mcp={mcp}) must source the trailer from rdm hook done-line"
            );
            assert!(
                content.contains("git commit --amend"),
                "skill-land (mcp={mcp}) must amend the synthesized trailer onto the branch tip"
            );
            // ...BEFORE the rebase/fast-forward, so landing needs no manual rebase.
            assert!(
                content.contains("**before** the rebase and fast-forward below"),
                "skill-land (mcp={mcp}) must amend BEFORE the rebase so no manual rebase is ever needed"
            );
            assert!(
                content.contains("never needs a manual rebase"),
                "skill-land (mcp={mcp}) must state that an autonomous branch never needs a manual rebase"
            );
            // A failed done-line is an abort, not an empty amend.
            assert!(
                content.contains("never amend an empty trailer"),
                "skill-land (mcp={mcp}) must abort rather than amend an empty trailer"
            );
        }
    }

    #[test]
    fn skill_autopilot_documents_loop_and_budgets() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[6].content;
        assert!(content.contains("name: rdm-autopilot"));
        // Drives one named roadmap; the slug is required and the loop never roams.
        assert!(content.contains("required roadmap slug"));
        assert!(content.contains("never roams to another roadmap"));
        assert!(content.contains("Workflow"));
        assert!(content.contains("rdm next --roadmap <slug> --format json"));
        // Composes the per-phase dispatch workflow rather than re-implementing it.
        assert!(content.contains("dispatch-phase"));
        // Bounded run: global step budget + budgets section + always-on summary.
        assert!(content.contains("global step budget"));
        assert!(content.contains("DEFAULT_GLOBAL_BUDGET"));
        assert!(content.contains("Run modes"));
        // Full prose-parity content: the known-good stop-reason allowlist and
        // the advance/park read-back confirmation loop, mirroring the local
        // dogfood skill's depth (the earlier placeholder sentence is gone).
        assert!(!content.contains("full prose-parity documentation lands in a follow-up phase"));
        assert!(
            content
                .contains("nothing`, `blocked-on-dependencies`, `budget`, `plan-only-exhausted`")
        );
        assert!(!content.contains("mechanical-model-unresolved"));
        assert!(content.contains("read it back"));
        assert!(content.contains("confirm `status` matches"));
        assert!(content.contains("up to **2** times total"));
        // Escalation rule is owned by the shared protocol, not redefined here;
        // the batch queue is surfaced via `rdm review blocked`.
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("rdm review blocked"));
        // No --land flag on autopilot; landing stays rdm-land's exclusive job.
        assert!(!content.contains("- `--land`"));
        assert!(content.contains("There is no `--land` flag here"));
        assert!(content.contains("never touched"));
        // Dry-run / bounded modes, including the two dispatch-phase budget overrides.
        assert!(content.contains("--plan-only"));
        assert!(content.contains("--max-phases"));
        assert!(content.contains("--max-plan-revise"));
        assert!(content.contains("--max-code-rework"));
        // Active driver: every dispatched phase actively runs review.
        assert!(content.contains("active driver"));
        // Never writes a Done: line by hand.
        assert!(!content.contains("Done: <roadmap-slug>/<phase-stem>"));
        // Unattended-permission guidance.
        assert!(content.contains("--permission-mode auto"));
        // The now-superseded Mandatory-dispatch / inline-collapse checklist is gone.
        assert!(!content.contains("Mandatory dispatch"));
        assert!(!content.contains("inline-collapse"));
        // generate_workflows() no longer ships an `autopilot.js` (2 files
        // remain: rdm-wf-dispatch-phase.js, rdm-wf-review-refute-fix.js), so
        // this template
        // must never instruct invoking a Workflow literally named
        // "autopilot" — that call would target a file this same generator
        // does not emit. It may still name the one real Workflow it composes
        // downstream (`rdm-wf-dispatch-phase`); the estimate pre-pass is
        // intentionally dropped from this distributed template (see
        // docs/workflow-vs-prose-boundary.md), so this template must never
        // instruct invoking the estimate engine either, under EITHER its
        // pre-rename bare name or its current `rdm-wf-` name.
        assert!(!content.contains("Invoke the `autopilot`"));
        assert!(!content.contains("the `autopilot` workflow"));
        assert!(!content.contains(".claude/workflows/autopilot.js"));
        assert!(!content.contains("Invoke the `estimate`"));
        assert!(!content.contains("the `estimate` Workflow"));
        assert!(!content.contains("Invoke the `rdm-wf-estimate`"));
        assert!(!content.contains("the `rdm-wf-estimate` Workflow"));
    }

    #[test]
    fn skill_dispatch_phase_documents_contract_and_plan_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[5].content;
        assert!(content.contains("name: rdm-dispatch-phase"));
        // It is a thin shim invoking the Workflow tool, not a prose 8-step loop.
        assert!(content.contains("thin shim"));
        assert!(content.contains("Workflow"));
        assert!(content.contains(".claude/workflows/rdm-wf-dispatch-phase.js"));
        // Task mode alongside phase mode.
        assert!(content.contains("--task"));
        assert!(content.contains("model tier"));
        // Structured OUTCOME with the three documented values plus the
        // status/writesCompletion/reason fields the canonical review stamps.
        assert!(content.contains("reviewed | rework | escalated"));
        assert!(content.contains("\"status\""));
        assert!(content.contains("\"writesCompletion\""));
        assert!(content.contains("\"reason\""));
        // A *separate*, independent plan-review stage gates the plan before
        // code is written, bounded to at most one revise round.
        assert!(content.contains("separate, independent plan-review"));
        assert!(content.contains("at most one revise round"));
        // Delegates code review to the canonical review pipeline (rdm-review's
        // source), which owns the Done: line via writesCompletion.
        assert!(content.contains("rdm-review"));
        assert!(content.contains("rdm-land"));
        // Escalation parks the phase as blocked; it never writes a Done: line.
        assert!(!content.contains("Done: <roadmap-slug>/<phase-stem>"));
        // Escalation follows the shared protocol and records a stage-tagged reason.
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("[plan]"));
        assert!(content.contains("[code]"));
        // The --permission-mode auto safety guardrail survives the rewrite.
        assert!(content.contains("--permission-mode auto"));
        assert!(content.contains("git stash -u"));
        assert!(content.contains("git reset --hard"));
        assert!(content.contains("git clean -fdx"));
        // The now-superseded Mandatory-dispatch / inline-collapse checklist is gone.
        assert!(!content.contains("Mandatory dispatch"));
        assert!(!content.contains("inline-collapse"));
    }

    #[test]
    fn skill_estimate_names_itself_and_uses_phase_update() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[4].content;
        assert!(content.contains("name: rdm-estimate"));
        assert!(content.contains("rdm phase update"));
        assert!(content.contains("--difficulty"));
    }

    #[test]
    fn skills_have_yaml_frontmatter() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        for skill in &skills {
            assert!(
                skill.content.starts_with("---\n"),
                "skill {} missing frontmatter",
                skill.relative_path
            );
            assert!(
                skill.content.contains("name:"),
                "skill {} missing name",
                skill.relative_path
            );
            assert!(
                skill.content.contains("allowed-tools:"),
                "skill {} missing allowed-tools",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skills_have_correct_names() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[0].content.contains("name: rdm-roadmap"));
        assert!(skills[1].content.contains("name: rdm-do"));
        assert!(skills[2].content.contains("name: rdm-review"));
        assert!(skills[3].content.contains("name: rdm-document"));
    }

    #[test]
    fn skills_use_project_flag() {
        let skills = generate_skills(&SkillOptions {
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: false,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("--project myproj"),
                "skill {} missing project flag",
                skill.relative_path
            );
            assert!(
                !skill.content.contains("<PROJECT>"),
                "skill {} has placeholder",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skills_use_placeholder_without_project() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("--project <PROJECT>"),
                "skill {} missing placeholder",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skills_contain_arguments_variable() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("$ARGUMENTS"),
                "skill {} missing $ARGUMENTS",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skill_roadmap_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[0].content;
        assert!(content.contains("rdm roadmap create"));
        assert!(content.contains("rdm phase create"));
        assert!(content.contains("rdm roadmap show"));
    }

    #[test]
    fn skill_do_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        // Phase flow commands.
        assert!(content.contains("rdm phase list"));
        assert!(content.contains("rdm phase show"));
        assert!(content.contains("rdm phase update"));
        // Task flow commands.
        assert!(content.contains("rdm task list"));
        assert!(content.contains("rdm task show"));
        assert!(content.contains("rdm task update"));
        assert!(content.contains("rdm task create"));
    }

    #[test]
    fn skills_include_principles_when_specified() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: Some("docs/principles.md".to_string()),
            mcp: false,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("## Principles"),
                "skill {} missing principles",
                skill.relative_path
            );
            assert!(
                skill.content.contains("docs/principles.md"),
                "skill {} missing principles path",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skills_exclude_principles_when_not_specified() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        for skill in &skills {
            assert!(
                !skill.content.contains("## Principles"),
                "skill {} has unexpected principles",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skill_do_has_write_edit_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("Write"));
        assert!(content.contains("Edit"));
    }

    #[test]
    fn skill_do_has_plan_mode_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("EnterPlanMode"));
        assert!(content.contains("ExitPlanMode"));
    }

    #[test]
    fn skill_do_uses_plan_mode_workflow() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("Enter plan mode"));
        assert!(content.contains("implementation plan"));
    }

    #[test]
    fn skill_do_runs_implementation_plan_review() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("rdm-plan-review"));
        assert!(content.contains("--implementation-plan"));
    }

    #[test]
    fn mcp_skill_do_runs_implementation_plan_review() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        assert!(content.contains("rdm-plan-review"));
        assert!(content.contains("--implementation-plan"));
    }

    #[test]
    fn skill_do_implementation_plan_review_precedes_approval_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        let plan_pos = content
            .find("Enter plan mode")
            .expect("missing Enter plan mode step");
        let review_pos = content
            .find("rdm-plan-review")
            .expect("missing rdm-plan-review reference");
        let approval_pos = content
            .find("Wait for user approval")
            .expect("missing Wait for user approval step");
        assert!(
            plan_pos < review_pos,
            "implementation-plan review step should come after drafting the plan"
        );
        assert!(
            review_pos < approval_pos,
            "implementation-plan review step should come before the approval gate"
        );
    }

    #[test]
    fn skill_do_auto_mode_folds_blocking_findings() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("--auto"));
        assert!(content.contains("blocking"));
        assert!(content.contains("fold every surviving"));
        assert!(content.contains("plan-review"));
    }

    #[test]
    fn mcp_skill_do_auto_mode_folds_blocking_findings() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        assert!(content.contains("--auto"));
        assert!(content.contains("blocking"));
        assert!(content.contains("fold every surviving"));
        assert!(content.contains("plan-review"));
    }

    #[test]
    fn skill_do_has_agent_tool() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let frontmatter = skills[1]
            .content
            .split("---")
            .nth(1)
            .expect("missing frontmatter");
        assert!(frontmatter.contains("Agent"));
    }

    #[test]
    fn mcp_skill_do_has_agent_tool() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let frontmatter = skills[1]
            .content
            .split("---")
            .nth(1)
            .expect("missing frontmatter");
        assert!(frontmatter.contains("Agent"));
    }

    #[test]
    fn skill_roadmap_no_write_edit_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        // Roadmap skill should only have Read, Bash, Glob, Grep
        let frontmatter = skills[0]
            .content
            .split("---")
            .nth(1)
            .expect("missing frontmatter");
        assert!(!frontmatter.contains("Write"));
        assert!(!frontmatter.contains("Edit"));
    }

    #[test]
    fn skill_roadmap_notes_plan_review_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[0].content;
        assert!(content.contains("needs-plan-review"));
        assert!(content.contains("plan_review"));
    }

    #[test]
    fn skill_review_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("rdm phase show"));
        assert!(content.contains("rdm task show"));
    }

    #[test]
    fn skill_review_has_correct_name() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[2].content.contains("name: rdm-review"));
    }

    #[test]
    fn skill_review_has_agent_tool() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("Agent"));
    }

    #[test]
    fn skill_review_contains_arguments_variable() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[2].content.contains("$ARGUMENTS"));
    }

    #[test]
    fn skill_review_categorizes_findings() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        // Small findings are fixed inline and amended; large findings are filed as tasks.
        assert!(content.contains("git commit --amend"));
        assert!(content.contains("rdm task create"));
        assert!(content.contains("Small"));
        assert!(content.contains("Large"));
    }

    #[test]
    fn skill_review_transitions_to_reviewed() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        // The three canonical outcomes and the statuses they map to.
        assert!(content.contains("reviewed"));
        assert!(content.contains("rework"));
        assert!(content.contains("escalated"));
        assert!(content.contains("`in-progress`"));
        assert!(content.contains("`blocked`"));
        // The completion trailer is never hand-typed: it is sourced from rdm.
        assert!(content.contains("rdm hook done-line"));
        assert!(content.contains("git commit --amend"));
    }

    #[test]
    fn skill_review_dispatches_adaptive_fleet() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("fleet"));
        // Every canonical dimension is documented, including the new security one.
        for dim in [
            "**ac**",
            "**correctness**",
            "**tests**",
            "**architecture**",
            "**api-docs**",
            "**changelog**",
            "**security**",
        ] {
            assert!(content.contains(dim), "missing dimension: {dim}");
        }
        // Conditional dimensions are gated on per-dimension triggers.
        assert!(content.contains("trigger"));
        // The fleet agents review without editing.
        assert!(content.contains("read-only"));
    }

    #[test]
    fn skill_review_has_refute_pass() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("Refute"));
        // The refuter returns a refuted boolean plus a corrected confidence.
        assert!(content.contains("`refuted` (boolean)"));
        // The finder is never the refuter.
        assert!(content.contains("never the agent that confirms it"));
        // Post-refutation confidence floor.
        assert!(content.contains("below **70**"));
    }

    #[test]
    fn skill_review_acts_by_finding_provenance() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        // Findings reach the act step with two provenances: refuter-verified,
        // and non-gating ones the pipeline passed through un-refuted. The
        // shipped skill must state both, and must NOT keep the retired
        // absolute that forbids acting on anything un-refuted.
        assert!(content.contains("un-refuted ones by disposition"));
        assert!(content.contains("graded and failed to refute"));
        assert!(content.contains("`unrefuted: true`"));
        assert!(content.contains("reported, not verified"));
        assert!(!content.contains("Never fix or file an unverified"));
    }

    #[test]
    fn skill_review_mcp_dispatches_adaptive_fleet() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        assert!(content.contains("fleet"));
        for dim in [
            "**ac**",
            "**correctness**",
            "**tests**",
            "**architecture**",
            "**api-docs**",
            "**changelog**",
            "**security**",
        ] {
            assert!(content.contains(dim), "missing dimension: {dim}");
        }
        assert!(content.contains("trigger"));
        assert!(content.contains("read-only"));
    }

    #[test]
    fn skill_review_mcp_has_refute_pass() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        assert!(content.contains("Refute"));
        assert!(content.contains("`refuted` (boolean)"));
        assert!(content.contains("never the agent that confirms it"));
        assert!(content.contains("below **70**"));
    }

    #[test]
    fn skill_review_mcp_acts_by_finding_provenance() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        // Findings reach the act step with two provenances: refuter-verified,
        // and non-gating ones the pipeline passed through un-refuted. The
        // shipped skill must state both, and must NOT keep the retired
        // absolute that forbids acting on anything un-refuted.
        assert!(content.contains("un-refuted ones by disposition"));
        assert!(content.contains("graded and failed to refute"));
        assert!(content.contains("`unrefuted: true`"));
        assert!(content.contains("reported, not verified"));
        assert!(!content.contains("Never fix or file an unverified"));
    }

    #[test]
    fn skill_review_has_blocked_verdict() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        // Severity scale drives the verdict.
        assert!(content.contains("Severity scale"));
        // The retired quartet is gone; the canonical trio has a strict order.
        assert!(!content.contains("PASS WITH CONCERNS"));
        assert!(!content.contains("**BLOCKED**"));
        assert!(content.contains("**escalated**"));
        assert!(content.contains("**rework**"));
        assert!(content.contains("**reviewed**"));
        assert!(content.contains("the first matching rule wins"));
        // Escalation maps to `blocked` for BOTH item kinds, with a [code] reason.
        assert!(content.contains("| **escalated** |"));
        assert!(!content.contains("tasks have no `blocked` status"));
        assert!(content.contains("`blocked` is a valid task status"));
        assert!(content.contains("[code]"));
    }

    #[test]
    fn skill_review_mcp_has_blocked_verdict() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        assert!(content.contains("Severity scale"));
        assert!(!content.contains("PASS WITH CONCERNS"));
        assert!(!content.contains("**BLOCKED**"));
        assert!(content.contains("**escalated**"));
        assert!(content.contains("**rework**"));
        assert!(content.contains("**reviewed**"));
        assert!(content.contains("the first matching rule wins"));
        // MCP gate uses the tool call with status "blocked" for an escalation.
        assert!(content.contains("status: \"blocked\""));
        assert!(!content.contains("tasks have no `blocked` status"));
        assert!(content.contains("`blocked` is a valid task status"));
    }

    #[test]
    fn skill_review_sizes_the_fleet_via_model_policy() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("rdm model resolve review-find"));
        assert!(content.contains("rdm model resolve review-verify"));
        assert!(content.contains("tier hint"));
        assert!(content.contains("small"));
        assert!(content.contains("large"));
        assert!(content.contains("never the inherited session model"));
    }

    #[test]
    fn skill_review_mcp_sizes_the_fleet_via_model_policy() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        assert!(content.contains("rdm model resolve review-find"));
        assert!(content.contains("rdm model resolve review-verify"));
        assert!(content.contains("tier hint"));
        assert!(content.contains("never the inherited session model"));
    }

    #[test]
    fn skill_document_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[3].content;
        assert!(content.contains("rdm roadmap show"));
        assert!(content.contains("rdm phase show"));
        assert!(content.contains("--format json"));
        assert!(content.contains("git log"));
        assert!(content.contains("git diff"));
    }

    #[test]
    fn skill_document_has_write_edit_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[3].content;
        assert!(content.contains("Write"));
        assert!(content.contains("Edit"));
    }

    #[test]
    fn skill_document_no_plan_mode_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let frontmatter = skills[3]
            .content
            .split("---")
            .nth(1)
            .expect("missing frontmatter");
        assert!(!frontmatter.contains("EnterPlanMode"));
        assert!(!frontmatter.contains("ExitPlanMode"));
    }

    #[test]
    fn skill_do_finalize_runs_canonical_review() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        // Finalize still stamps the transient needs-review marker via the update
        // command...
        assert!(content.contains("--status needs-review"));
        // ...but it no longer PARKS there: it actively invokes the canonical
        // review (the `rdm-review` skill, the projection of the one review
        // source) as part of finalizing.
        assert!(content.contains("Immediately invoke the `rdm-review` skill"));
        // The review runs in BOTH lanes — interactive and --auto — not just one.
        assert!(content.contains("This runs in **both** modes"));
        assert!(
            content
                .contains("`--auto` (which skips only the human confirmation, never the review)")
        );
        // The completion trailer is sourced from rdm, never hand-typed...
        assert!(content.contains("rdm hook done-line"));
        assert!(content.contains("Never hand-type the completion trailer"));
        // ...so the raw format string never appears in the shipped skill.
        assert!(!content.contains("<roadmap-slug>/<phase-stem>"));
        // The stale "park it and let a hook pick it up later" framing is gone.
        assert!(!content.contains("deferred two-stage"));
        assert!(!content.contains("the sentinel that signals a review is pending"));
    }

    #[test]
    fn skill_do_uses_worktree_and_run_modes() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        // Work happens in one worktree per roadmap, created via the roadmap-scoped
        // worktree command (not a per-phase ref)...
        assert!(content.contains("worktree add <slug>"));
        // ...detecting the current worktree's roadmap via `worktree current`...
        assert!(content.contains("worktree current"));
        // ...and following the Match/None/Mismatch in-place flow.
        assert!(content.contains("**Match**"));
        assert!(content.contains("**None**"));
        assert!(content.contains("**Mismatch**"));
        assert!(content.contains("work in place"));
        // EnterWorktree is a one-time convenience, NOT a correctness dependency:
        // non-Claude hosts have a working cd/launch entry path.
        assert!(content.contains("EnterWorktree"));
        assert!(content.contains("not** a correctness dependency"));
        assert!(content.contains("cd`/launch"));
        // Tasks keep their own per-task worktree.
        assert!(content.contains("worktree add task/<slug>"));
        // The Mismatch branch pins the relaunch entry path and the reason
        // EnterWorktree cannot be used from inside another worktree — so a
        // regression that dropped/inverted this caveat would fail, not pass.
        assert!(content.contains("relaunch"));
        assert!(content.contains(".claude/worktrees/"));
        // ...and the skill supports a non-interactive run mode...
        assert!(content.contains("--auto"));
        // ...with unattended-permission guidance for Claude Code.
        assert!(content.contains("--permission-mode auto"));
    }

    #[test]
    fn planning_workflow_includes_done_convention() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("Done:"));
        assert!(content.contains("<roadmap-slug>/<phase-stem>"));
    }

    // --- MCP config generation tests ---

    #[test]
    fn mcp_config_is_valid_json() {
        let output = generate_mcp_config(&McpConfigOptions { root: None });
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("invalid JSON");
        assert!(parsed.is_object());
    }

    #[test]
    fn mcp_config_without_root() {
        let output = generate_mcp_config(&McpConfigOptions { root: None });
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let args = parsed["mcpServers"]["rdm"]["args"]
            .as_array()
            .expect("args should be array");
        assert_eq!(args.len(), 1);
        assert_eq!(args[0], "mcp");
    }

    #[test]
    fn mcp_config_with_root() {
        let output = generate_mcp_config(&McpConfigOptions {
            root: Some("/home/user/plans".to_string()),
        });
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let args = parsed["mcpServers"]["rdm"]["args"]
            .as_array()
            .expect("args should be array");
        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "--root");
        assert_eq!(args[1], "/home/user/plans");
        assert_eq!(args[2], "mcp");
    }

    #[test]
    fn mcp_config_has_correct_structure() {
        let output = generate_mcp_config(&McpConfigOptions { root: None });
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["mcpServers"]["rdm"]["command"], "rdm");
        assert!(parsed["mcpServers"]["rdm"]["args"].is_array());
    }

    // --- MCP agent instructions tests ---

    #[test]
    fn mcp_agent_config_references_mcp_tools() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("rdm_roadmap_list"));
        assert!(content.contains("rdm_task_list"));
        assert!(content.contains("rdm_roadmap_show"));
        assert!(content.contains("rdm_phase_show"));
        assert!(content.contains("rdm_task_show"));
        assert!(content.contains("rdm_phase_update"));
        assert!(content.contains("rdm_task_update"));
        assert!(content.contains("rdm_roadmap_create"));
        assert!(content.contains("rdm_phase_create"));
        assert!(content.contains("rdm_task_create"));
        assert!(content.contains("rdm_search"));
    }

    #[test]
    fn mcp_agent_config_no_bash_blocks() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(
            !content.contains("```bash"),
            "MCP instructions should not contain bash code blocks"
        );
    }

    #[test]
    fn mcp_agent_config_has_key_sections() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("# rdm"));
        assert!(content.contains("## Setup"));
        assert!(content.contains("## Discovering work"));
        assert!(content.contains("## Reading details"));
        assert!(content.contains("## Searching"));
        assert!(content.contains("## Updating status"));
        assert!(content.contains("## Creating items"));
        assert!(content.contains("## Planning workflow"));
        assert!(content.contains("## Status transitions"));
    }

    #[test]
    fn mcp_agent_config_with_project() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("\"myproj\""));
        assert!(!content.contains("<PROJECT>"));
    }

    #[test]
    fn mcp_agent_config_without_project() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("\"<PROJECT>\""));
    }

    #[test]
    fn mcp_agent_config_no_no_edit() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(
            !content.contains("--no-edit"),
            "MCP instructions should not mention --no-edit"
        );
    }

    #[test]
    fn mcp_agent_config_includes_done_convention() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("Done:"));
        assert!(content.contains("<roadmap-slug>/<phase-stem>"));
    }

    #[test]
    fn mcp_agent_config_includes_promote() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("rdm_task_promote"));
    }

    #[test]
    fn mcp_agent_config_cursor_has_frontmatter() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::Cursor,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.starts_with("---\n"));
        assert!(content.contains("rdm_roadmap_list"));
    }

    #[test]
    fn mcp_agent_config_principles_included() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: Some("docs/principles.md".to_string()),
            mcp: true,
        });
        assert!(content.contains("## Principles"));
        assert!(content.contains("docs/principles.md"));
    }

    // --- MCP skill generation tests ---

    #[test]
    fn mcp_skills_returns_eleven_files() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        // Workflows are a separate emission surface (see `generate_workflows`)
        // and are not counted here, and are not MCP/CLI-flavored anyway.
        // Was 10 (no `rdm-backlog` MCP twin) before `skill_backlog_mcp` was
        // added — now matches the cli branch's count (see
        // `generate_skills_cli_mcp_name_parity`).
        assert_eq!(skills.len(), 11);
    }

    #[test]
    fn mcp_skills_correct_paths() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert_eq!(skills[0].relative_path, "rdm-roadmap/SKILL.md");
        assert_eq!(skills[1].relative_path, "rdm-do/SKILL.md");
        assert_eq!(skills[2].relative_path, "rdm-review/SKILL.md");
        assert_eq!(skills[3].relative_path, "rdm-document/SKILL.md");
        assert_eq!(skills[4].relative_path, "rdm-estimate/SKILL.md");
        assert_eq!(skills[5].relative_path, "rdm-dispatch-phase/SKILL.md");
        assert_eq!(skills[6].relative_path, "rdm-autopilot/SKILL.md");
        assert_eq!(skills[7].relative_path, "rdm-land/SKILL.md");
        assert_eq!(skills[8].relative_path, "rdm-revise/SKILL.md");
        assert_eq!(skills[9].relative_path, "rdm-plan-review/SKILL.md");
        assert_eq!(skills[10].relative_path, "rdm-backlog/SKILL.md");
    }

    #[test]
    fn mcp_skill_land_uses_mcp_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[7].content;
        assert!(content.contains("name: rdm-land"));
        assert!(content.contains("$ARGUMENTS"));
        // Same landing contract as the CLI variant.
        assert!(content.contains("merge --ff-only"));
        assert!(content.contains("linear history"));
        assert!(content.contains("reviewed"));
        assert!(content.contains("Done:"));
        assert!(content.contains("reviewed → done"));
        assert!(content.contains("git rebase --abort"));
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("never auto-lands"));
        // Status reads/updates and single-worktree cleanup go through MCP tools.
        assert!(content.contains("rdm_phase_show"));
        assert!(content.contains("rdm_phase_update"));
        assert!(content.contains("rdm_worktree_remove"));
        // Batch prune has no MCP tool this phase — delegated as the CLI command.
        assert!(content.contains("rdm worktree prune"));
        // The git landing is delegated to a Bash-capable subagent via Agent.
        assert!(content.contains("Agent"));
        assert!(content.contains("subagent"));
        // MCP variant: Bash-free frontmatter, mcp__rdm__ tools resolved.
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(!frontmatter.contains("  - Bash"));
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_show"));
        assert!(frontmatter.contains("mcp__rdm__rdm_worktree_remove"));
    }

    #[test]
    fn mcp_skill_autopilot_uses_mcp_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[6].content;
        assert!(content.contains("name: rdm-autopilot"));
        // Drives one named roadmap; the slug is required and the loop never roams.
        assert!(content.contains("required roadmap slug"));
        assert!(content.contains("never roams to another roadmap"));
        // The MCP `rdm_next` tool is named in the body wherever the loop
        // driver is described.
        assert!(content.contains("rdm_next"));
        // Composes the per-phase dispatch workflow.
        assert!(content.contains("dispatch-phase"));
        // Bounded run + run-modes section + shared escalation protocol + batch queue.
        assert!(content.contains("global step budget"));
        assert!(content.contains("DEFAULT_GLOBAL_BUDGET"));
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("rdm review blocked"));
        // Full prose-parity content: the known-good stop-reason allowlist and
        // the advance/park read-back confirmation loop via the new
        // rdm_phase_show tool (the earlier placeholder sentence is gone).
        assert!(!content.contains("full prose-parity documentation lands in a follow-up phase"));
        assert!(
            content
                .contains("nothing`, `blocked-on-dependencies`, `budget`, `plan-only-exhausted`")
        );
        assert!(!content.contains("mechanical-model-unresolved"));
        assert!(content.contains("mcp__rdm__rdm_phase_show"));
        assert!(content.contains("up to **2** times total"));
        // Regression: the advance and park call sites must use the same
        // `project`/`roadmap`/`phase` argument shape every other MCP template
        // uses (PhaseUpdateParams/PhaseParams on the server side), never the
        // wrong `stem` field name or a missing `project`.
        assert!(content.contains(
            "call `mcp__rdm__rdm_phase_update` with `project: \"<PROJECT>\", roadmap: \"<slug>\", phase: S, status: <OUTCOME.status || \"reviewed\">`"
        ));
        assert!(content.contains(
            "call `mcp__rdm__rdm_phase_show` with `project: \"<PROJECT>\", roadmap: \"<slug>\", phase: S` and confirm `status` matches"
        ));
        assert!(content.contains(
            "call `mcp__rdm__rdm_phase_update` with `project: \"<PROJECT>\", roadmap: \"<slug>\", phase: S, status: \"blocked\", reason: \"<reason>\"`"
        ));
        assert!(content.contains(
            "call `mcp__rdm__rdm_phase_show` with `project: \"<PROJECT>\", roadmap: \"<slug>\", phase: S` and confirm `status: \"blocked\"`"
        ));
        assert!(!content.contains("with `stem: S,"));
        // No --land flag; dry-run / bounded modes including the two
        // dispatch-phase budget overrides.
        assert!(!content.contains("- `--land`"));
        assert!(content.contains("There is no `--land` flag here"));
        assert!(content.contains("--plan-only"));
        assert!(content.contains("--max-phases"));
        assert!(content.contains("--max-plan-revise"));
        assert!(content.contains("--max-code-rework"));
        // MCP variant: Workflow-only frontmatter, mcp__rdm__ tools resolved.
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(!frontmatter.contains("  - Bash"));
        assert!(frontmatter.contains("mcp__rdm__rdm_next"));
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_update"));
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_show"));
        // The distributed template no longer hoists a phase list: there is no
        // `estimate` pre-pass downstream to feed it (see below), so the
        // `rdm_phase_list` tool is neither allowed nor referenced.
        assert!(!frontmatter.contains("mcp__rdm__rdm_phase_list"));
        assert!(!content.contains("mcp__rdm__rdm_phase_list"));
        assert!(!content.contains("phaseList"));
        // The now-superseded Mandatory-dispatch / inline-collapse checklist is gone.
        assert!(!content.contains("Mandatory dispatch"));
        assert!(!content.contains("inline-collapse"));
        // generate_workflows() no longer ships an `autopilot.js` (2 files
        // remain: rdm-wf-dispatch-phase.js, rdm-wf-review-refute-fix.js), so
        // this template
        // must never instruct invoking a Workflow literally named
        // "autopilot" — that call would target a file this same generator
        // does not emit. It may still name the one real Workflow it composes
        // downstream (`rdm-wf-dispatch-phase`); the estimate pre-pass is
        // intentionally dropped from this distributed template (see
        // docs/workflow-vs-prose-boundary.md), so this template must never
        // instruct invoking the estimate engine either, under EITHER its
        // pre-rename bare name or its current `rdm-wf-` name.
        assert!(!content.contains("Invoke the `autopilot`"));
        assert!(!content.contains("the `autopilot` workflow"));
        assert!(!content.contains(".claude/workflows/autopilot.js"));
        assert!(!content.contains("Invoke the `estimate`"));
        assert!(!content.contains("the `estimate` Workflow"));
        assert!(!content.contains("Invoke the `rdm-wf-estimate`"));
        assert!(!content.contains("the `rdm-wf-estimate` Workflow"));
    }

    #[test]
    fn mcp_skill_dispatch_phase_uses_mcp_tools_and_plan_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[5].content;
        assert!(content.contains("name: rdm-dispatch-phase"));
        // Thin shim invoking the Workflow tool, not per-tool MCP wiring — the
        // workflow itself performs the worktree/status operations internally.
        assert!(content.contains("thin shim"));
        assert!(content.contains(".claude/workflows/rdm-wf-dispatch-phase.js"));
        assert!(content.contains("--task"));
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(frontmatter.contains("Workflow"));
        assert!(!frontmatter.contains("  - Bash"));
        assert!(!frontmatter.contains("  - Edit"));
        assert!(!frontmatter.contains("  - Write"));
        // The in-progress stamp hoist: the shim stamps the item itself and
        // passes `alreadyInProgress: true`, so the workflow skips its own
        // stamp subagent. Both update tools must be allowed and resolved.
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_update"));
        assert!(frontmatter.contains("mcp__rdm__rdm_task_update"));
        assert!(content.contains("mcp__rdm__rdm_phase_update"));
        assert!(content.contains("mcp__rdm__rdm_task_update"));
        assert!(content.contains("alreadyInProgress"));
        // Same bounded, independent plan gate and structured outcome as the CLI variant.
        assert!(content.contains("separate, independent plan-review"));
        assert!(content.contains("at most one revise round"));
        assert!(content.contains("reviewed | rework | escalated"));
        assert!(content.contains("\"status\""));
        assert!(content.contains("\"writesCompletion\""));
        assert!(content.contains("rdm-review"));
        assert!(content.contains("rdm-land"));
        // Escalation follows the shared protocol and records a stage-tagged reason.
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("[plan]"));
        assert!(content.contains("[code]"));
        // The --permission-mode auto safety guardrail survives the rewrite.
        assert!(content.contains("--permission-mode auto"));
        assert!(content.contains("git stash -u"));
        // The now-superseded Mandatory-dispatch / inline-collapse checklist is gone.
        assert!(!content.contains("Mandatory dispatch"));
        assert!(!content.contains("inline-collapse"));
    }

    #[test]
    fn mcp_skill_estimate_names_itself_and_uses_phase_update() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[4].content;
        assert!(content.contains("name: rdm-estimate"));
        assert!(content.contains("rdm_phase_update"));
        // MCP skill must not list Bash in allowed-tools.
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(!frontmatter.contains("Bash"));
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_update"));
    }

    #[test]
    fn mcp_skills_no_bash_in_allowed_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        for skill in &skills {
            // Extract frontmatter (between first and second ---)
            let parts: Vec<&str> = skill.content.splitn(3, "---").collect();
            let frontmatter = parts[1];
            assert!(
                !frontmatter.contains("  - Bash"),
                "MCP skill {} should not list Bash in allowed-tools",
                skill.relative_path
            );
        }
    }

    #[test]
    fn mcp_skills_have_mcp_tools_in_allowed_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("mcp__rdm__"),
                "MCP skill {} should list mcp__rdm__ tools in allowed-tools",
                skill.relative_path
            );
        }
    }

    /// Every `{t_*}` tool placeholder in an MCP skill template must have a
    /// matching entry in that skill's substitution list. A dropped tuple leaves
    /// the literal placeholder text in the shipped skill — including in its
    /// `allowed-tools` frontmatter — which no content-presence assertion
    /// catches, because unrelated placeholders still substitute fine.
    #[test]
    fn mcp_skills_substitute_every_tool_placeholder() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        for skill in &skills {
            assert!(
                !skill.content.contains("{t_"),
                "MCP skill {} ships an unsubstituted tool placeholder: {}",
                skill.relative_path,
                skill
                    .content
                    .lines()
                    .find(|line| line.contains("{t_"))
                    .unwrap_or_default()
                    .trim()
            );
        }
    }

    #[test]
    fn mcp_skills_reference_mcp_tool_calls() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        // Roadmap skill should reference MCP create tools
        assert!(skills[0].content.contains("rdm_roadmap_create"));
        assert!(skills[0].content.contains("rdm_phase_create"));
        // Do skill should reference both MCP phase and task tools
        assert!(skills[1].content.contains("rdm_phase_list"));
        assert!(skills[1].content.contains("rdm_phase_show"));
        assert!(skills[1].content.contains("rdm_phase_update"));
        assert!(skills[1].content.contains("rdm_task_list"));
        assert!(skills[1].content.contains("rdm_task_show"));
        assert!(skills[1].content.contains("rdm_task_update"));
        // ...and the worktree tool that isolates its work.
        assert!(skills[1].content.contains("rdm_worktree_add"));
    }

    #[test]
    fn mcp_skills_have_correct_names() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(skills[0].content.contains("name: rdm-roadmap"));
        assert!(skills[1].content.contains("name: rdm-do"));
        assert!(skills[2].content.contains("name: rdm-review"));
        assert!(skills[3].content.contains("name: rdm-document"));
    }

    #[test]
    fn mcp_skill_roadmap_notes_plan_review_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[0].content;
        assert!(content.contains("needs-plan-review"));
        assert!(content.contains("plan_review"));
    }

    #[test]
    fn mcp_skills_use_project_param() {
        let skills = generate_skills(&SkillOptions {
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: true,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("\"myproj\""),
                "MCP skill {} should use project param",
                skill.relative_path
            );
        }
    }

    #[test]
    fn mcp_skills_contain_arguments_variable() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("$ARGUMENTS"),
                "MCP skill {} missing $ARGUMENTS",
                skill.relative_path
            );
        }
    }

    #[test]
    fn mcp_skill_do_finalize_runs_canonical_review() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        // Finalize still stamps the transient needs-review marker via the update
        // tool...
        assert!(content.contains("status: \"needs-review\""));
        // ...but it no longer PARKS there: it actively invokes the canonical
        // review as part of finalizing.
        assert!(content.contains("Immediately invoke the `rdm-review` skill"));
        // The review runs in BOTH lanes — interactive and --auto — not just one.
        assert!(content.contains("This runs in **both** modes"));
        assert!(
            content
                .contains("`--auto` (which skips only the human confirmation, never the review)")
        );
        // The completion trailer is sourced from rdm, never hand-typed...
        assert!(content.contains("rdm hook done-line"));
        assert!(content.contains("Never hand-type the completion trailer"));
        // ...so the raw format string never appears in the shipped skill.
        assert!(!content.contains("<roadmap-slug>/<phase-stem>"));
        // The stale "park it and let a hook pick it up later" framing is gone.
        assert!(!content.contains("deferred two-stage"));
        assert!(!content.contains("the sentinel that signals a review is pending"));
    }

    #[test]
    fn mcp_skill_do_supports_run_modes_and_worktree() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        // The MCP variant supports the interactive/non-interactive run modes...
        assert!(content.contains("--auto"));
        assert!(content.contains("Run modes"));
        // ...and drives the one-worktree-per-roadmap, work-in-place flow via the
        // MCP worktree tools (not Bash, no EnterWorktree): it detects the current
        // worktree with rdm_worktree_current and creates/reuses the roadmap
        // worktree with rdm_worktree_add, following Match/None/Mismatch.
        assert!(content.contains("rdm_worktree_current"));
        assert!(content.contains("rdm_worktree_add"));
        assert!(content.contains("**Match**"));
        assert!(content.contains("**None**"));
        assert!(content.contains("**Mismatch**"));
        assert!(content.contains("work in place"));
        // The roadmap-scoped item ref is used, plus the per-task ref for tasks.
        assert!(content.contains("item: \"<slug>\""));
        assert!(content.contains("item: \"task/<slug>\""));
        // MCP hosts cd/open the returned path — EnterWorktree is not used here.
        assert!(!content.contains("EnterWorktree"));
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(frontmatter.contains("mcp__rdm__rdm_worktree_current"));
        assert!(frontmatter.contains("mcp__rdm__rdm_worktree_add"));
    }

    #[test]
    fn mcp_skill_do_implementation_plan_review_precedes_approval_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        let plan_pos = content
            .find("Enter plan mode")
            .expect("missing Enter plan mode step");
        let review_pos = content
            .find("rdm-plan-review")
            .expect("missing rdm-plan-review reference");
        let approval_pos = content
            .find("Wait for user approval")
            .expect("missing Wait for user approval step");
        assert!(
            plan_pos < review_pos,
            "implementation-plan review step should come after drafting the plan"
        );
        assert!(
            review_pos < approval_pos,
            "implementation-plan review step should come before the approval gate"
        );
    }

    #[test]
    fn mcp_skill_review_categorizes_findings() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        // Large findings are filed as tasks via the MCP create tool.
        assert!(content.contains("rdm_task_create"));
        assert!(content.contains("Small"));
        assert!(content.contains("Large"));
        assert!(content.contains("git commit --amend"));
    }

    #[test]
    fn mcp_skill_review_drives_transition() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        // Transition is driven via the MCP update tools; the status comes from
        // the generated outcome->status mapping (reviewed|in-progress|blocked).
        assert!(content.contains("rdm_phase_update"));
        assert!(content.contains("rdm_task_update"));
        assert!(content.contains("status: \"<status>\""));
        assert!(content.contains("`reviewed`, `in-progress`, or `blocked`"));
        // allowed-tools frontmatter lists the new MCP tools.
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_update"));
        assert!(frontmatter.contains("mcp__rdm__rdm_task_update"));
    }

    /// Both review skill variants share the generated review specification:
    /// the canonical outcome vocabulary, the seven-dimension fleet including
    /// the new `security` dimension, and a completion trailer sourced from
    /// `rdm hook done-line` rather than a hand-typed format string.
    #[test]
    fn skill_review_shares_the_generated_spec_across_variants() {
        for mcp in [false, true] {
            let skills = generate_skills(&SkillOptions {
                project: None,
                principles_file: None,
                mcp,
            });
            let content = &skills[2].content;
            for needle in [
                "**security**",
                "**reviewed**",
                "**rework**",
                "**escalated**",
                "rdm hook done-line",
                "`blocked` is a valid task status",
            ] {
                assert!(content.contains(needle), "mcp={mcp}: missing {needle}");
            }
            for retired in ["PASS WITH CONCERNS", "**BLOCKED**", "**FAIL**"] {
                assert!(
                    !content.contains(retired),
                    "mcp={mcp}: retired verdict word still present: {retired}"
                );
            }
        }
    }

    /// Every review skill variant carries the dimension/severity/verdict
    /// *definitions* solely inside `scripts/gen-skill-review.sh`'s generated
    /// `<!-- rdm:review-spec:begin -->` / `<!-- rdm:review-spec:end -->` span.
    /// Definitional sentences drawn from the generated block must not also be
    /// hand-duplicated in the surrounding Setup/Report/Act/Gate narrative —
    /// that narrative may still *reference* a term (e.g. a `--status
    /// reviewed` command literal, or "per the verdict below"), which is why
    /// this checks full defining phrases rather than the bare dimension /
    /// severity / verdict words, which legitimately recur outside the block.
    #[test]
    fn skill_review_spec_confined_to_generated_block() {
        const BEGIN: &str = "<!-- rdm:review-spec:begin";
        const END: &str = "<!-- rdm:review-spec:end -->";
        // Full definitional phrases lifted verbatim from the generated block:
        // the severity scale's three definitions, the dimensions intro, and
        // the verdict decision rule. A bare word like "blocking" or
        // "escalated" is *not* included here — those legitimately recur as
        // command literals / status values in the Gate section outside the
        // block.
        const DEFINITIONAL_PHRASES: &[&str] = &[
            // Severity scale definitions.
            "the work must not advance as-is",
            "recorded but non-gating",
            "minor optional improvement",
            // Dimensions intro (defines what "always-on"/"triggered" mean).
            "Scale the fleet to what the change actually touches",
            // Verdict decision rule.
            "Determine the outcome in this strict order",
            "needs a *human decision*",
        ];

        for mcp in [false, true] {
            let skills = generate_skills(&SkillOptions {
                project: None,
                principles_file: None,
                mcp,
            });
            let content = &skills[2].content;
            let begin = content
                .find(BEGIN)
                .unwrap_or_else(|| panic!("mcp={mcp}: missing begin marker"));
            let end = content
                .find(END)
                .unwrap_or_else(|| panic!("mcp={mcp}: missing end marker"));
            assert!(
                begin < end,
                "mcp={mcp}: begin marker must precede end marker"
            );

            let before = &content[..begin];
            let after = &content[end + END.len()..];

            for phrase in DEFINITIONAL_PHRASES {
                assert!(
                    !before.contains(phrase),
                    "mcp={mcp}: spec-definition phrase found before the generated block: {phrase:?}"
                );
                assert!(
                    !after.contains(phrase),
                    "mcp={mcp}: spec-definition phrase found after the generated block: {phrase:?}"
                );
            }
        }
    }

    // --- Raw `--skills` emission baseline ---
    //
    // A per-file sha256 fingerprint of everything the RAW emission surface
    // produces, captured from the pre-change tree and committed as a fixture
    // BEFORE plugin-mode emission existed. Its whole purpose is to be a
    // baseline that the plugin-mode emitter provably did not author: it makes
    // "raw `--skills` output is unchanged" a checkable claim rather than an
    // assertion of intent.

    /// The four canonical [`SkillOptions`] combinations the raw-emission
    /// baseline covers, each paired with the key prefix it contributes to the
    /// fixture map: the CLI and MCP surfaces, each with and without a project
    /// name and a principles file (the only two axes `generate_skills`
    /// substitutes on).
    fn raw_baseline_combos() -> Vec<(&'static str, SkillOptions)> {
        vec![
            (
                "cli-bare",
                SkillOptions {
                    project: None,
                    principles_file: None,
                    mcp: false,
                },
            ),
            (
                "cli-full",
                SkillOptions {
                    project: Some("demo".to_string()),
                    principles_file: Some("PRINCIPLES.md".to_string()),
                    mcp: false,
                },
            ),
            (
                "mcp-bare",
                SkillOptions {
                    project: None,
                    principles_file: None,
                    mcp: true,
                },
            ),
            (
                "mcp-full",
                SkillOptions {
                    project: Some("demo".to_string()),
                    principles_file: Some("PRINCIPLES.md".to_string()),
                    mcp: true,
                },
            ),
        ]
    }

    /// Recomputes the `"<combo-key>/<relative_path>" -> "<sha256-hex>"` map
    /// over the raw emission surface: [`generate_skills`] across all four
    /// [`raw_baseline_combos`], plus [`generate_workflows`].
    fn raw_emission_checksums() -> std::collections::BTreeMap<String, String> {
        let mut map = std::collections::BTreeMap::new();
        for (key, opts) in raw_baseline_combos() {
            for skill in generate_skills(&opts) {
                map.insert(
                    format!("{key}/{}", skill.relative_path),
                    sha256_hex(skill.content.as_bytes()),
                );
            }
        }
        for workflow in generate_workflows() {
            map.insert(
                format!("workflows/{}", workflow.relative_path),
                sha256_hex(workflow.content.as_bytes()),
            );
        }
        map
    }

    fn raw_baseline_fixture_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/raw-skills-baseline.json")
    }

    /// Regenerates the committed raw-emission baseline fixture.
    ///
    /// Deliberately `#[ignore]`d: running it rewrites the very artifact
    /// [`raw_skills_emission_matches_committed_baseline`] checks against, so
    /// an accidental run would launder a real regression into a "new
    /// baseline". If the raw templates ever legitimately change, regenerating
    /// this fixture must be a deliberate, separately-reviewed commit that
    /// stands on its own — never bundled with the change that moved the
    /// bytes.
    ///
    /// Run with:
    /// `cargo test -p rdm-core regenerate_raw_skills_baseline -- --ignored`
    #[test]
    #[ignore = "generator: rewrites the committed baseline fixture; run deliberately only"]
    fn regenerate_raw_skills_baseline() {
        let map = raw_emission_checksums();
        let path = raw_baseline_fixture_path();
        std::fs::create_dir_all(path.parent().expect("fixture path has a parent"))
            .expect("create fixture dir");
        let json = serde_json::to_string_pretty(&map).expect("serialize baseline");
        std::fs::write(&path, format!("{json}\n")).expect("write baseline fixture");
    }

    #[test]
    fn raw_skills_emission_matches_committed_baseline() {
        let path = raw_baseline_fixture_path();
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "failed to read {}: {e}\n\
                 regenerate with: cargo test -p rdm-core regenerate_raw_skills_baseline -- --ignored",
                path.display()
            )
        });
        let expected: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&raw).expect("baseline fixture is valid JSON");
        let actual = raw_emission_checksums();

        assert!(
            !expected.is_empty(),
            "baseline fixture is empty — it would pass vacuously"
        );

        let mut differing: Vec<String> = Vec::new();
        for (key, want) in &expected {
            match actual.get(key) {
                Some(got) if got == want => {}
                Some(got) => differing.push(format!("{key}: baseline {want} != emitted {got}")),
                None => differing.push(format!("{key}: present in baseline, absent from emission")),
            }
        }
        for key in actual.keys() {
            if !expected.contains_key(key) {
                differing.push(format!("{key}: emitted but absent from baseline"));
            }
        }
        assert!(
            differing.is_empty(),
            "raw `--skills` emission drifted from the committed pre-change baseline:\n  {}",
            differing.join("\n  ")
        );
    }

    // --- Plugin-layout emission tests ---

    /// The two `SkillOptions` surfaces, keyed by the `mcp` flag, used by the
    /// plugin tests that must hold on both.
    fn plugin_test_opts(mcp: bool) -> SkillOptions {
        SkillOptions {
            project: Some("demo".to_string()),
            principles_file: None,
            mcp,
        }
    }

    /// Returns the multiset of maximal `rdm-`-prefixed kebab tokens in `body`,
    /// using exactly the token rule [`rewrite_skill_names`] applies (preceding
    /// byte not in `[A-Za-z0-9_-]`; maximal `[A-Za-z0-9-]` run; trailing `-`
    /// trimmed). Counting tokens rather than substrings is what lets the
    /// preservation assertions be exact: `rdm-do` can never be counted inside
    /// `rdm-document`.
    fn rdm_tokens(body: &str) -> std::collections::BTreeMap<&str, usize> {
        const NEEDLE: &[u8] = b"rdm-";
        let bytes = body.as_bytes();
        let mut counts = std::collections::BTreeMap::new();
        let mut i = 0usize;
        while i + NEEDLE.len() <= bytes.len() {
            if &bytes[i..i + NEEDLE.len()] != NEEDLE
                || (i > 0 && is_token_boundary_blocker(bytes[i - 1]))
            {
                i += 1;
                continue;
            }
            let mut end = i + NEEDLE.len();
            while end < bytes.len() && is_token_byte(bytes[end]) {
                end += 1;
            }
            let mut token_end = end;
            while token_end > i && bytes[token_end - 1] == b'-' {
                token_end -= 1;
            }
            *counts.entry(&body[i..token_end]).or_insert(0usize) += 1;
            i = end;
        }
        counts
    }

    /// Returns the multiset of ALL maximal kebab tokens in `body` (not just
    /// `rdm-`-prefixed ones), used for the rename token-delta check.
    fn kebab_tokens(body: &str) -> std::collections::BTreeMap<&str, usize> {
        let bytes = body.as_bytes();
        let mut counts = std::collections::BTreeMap::new();
        let mut i = 0usize;
        while i < bytes.len() {
            if !is_token_byte(bytes[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && is_token_byte(bytes[i]) {
                i += 1;
            }
            let token = body[start..i].trim_matches('-');
            if !token.is_empty() {
                *counts.entry(token).or_insert(0usize) += 1;
            }
        }
        counts
    }

    fn count_of(counts: &std::collections::BTreeMap<&str, usize>, token: &str) -> usize {
        counts.get(token).copied().unwrap_or(0)
    }

    /// Parses the `meta.name` a Workflow script declares.
    fn parse_meta_name(script: &str) -> String {
        let start = script
            .find("export const meta = {")
            .expect("workflow script declares `export const meta = {`");
        let line = script[start..]
            .lines()
            .find(|line| line.trim_start().starts_with("name:"))
            .expect("meta block declares a `name:`");
        line.trim()
            .trim_start_matches("name:")
            .trim()
            .trim_matches(|c| c == ',' || c == '\'')
            .to_string()
    }

    #[test]
    fn plugin_files_have_expected_layout() {
        for mcp in [false, true] {
            let opts = plugin_test_opts(mcp);
            let files = generate_plugin_files(&opts);
            let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
            assert_eq!(
                paths,
                vec![
                    ".claude-plugin/plugin.json",
                    "skills/roadmap/SKILL.md",
                    "skills/do/SKILL.md",
                    "skills/review/SKILL.md",
                    "skills/document/SKILL.md",
                    "skills/estimate/SKILL.md",
                    "skills/dispatch-phase/SKILL.md",
                    "skills/autopilot/SKILL.md",
                    "skills/land/SKILL.md",
                    "skills/revise/SKILL.md",
                    "skills/plan-review/SKILL.md",
                    "skills/backlog/SKILL.md",
                    "workflows/rdm-wf-dispatch-phase.js",
                    "workflows/rdm-wf-review-refute-fix.js",
                ],
                "mcp={mcp}"
            );
            assert_eq!(files.len(), 14, "mcp={mcp}");

            for path in &paths {
                let p = std::path::Path::new(path);
                assert!(p.is_relative(), "mcp={mcp}: {path} is not relative");
                assert!(
                    !p.components()
                        .any(|c| matches!(c, std::path::Component::ParentDir)),
                    "mcp={mcp}: {path} contains a `..` component"
                );
                assert!(
                    !path.starts_with(".claude-plugin/workflows")
                        && !path.starts_with(".claude-plugin/skills"),
                    "mcp={mcp}: {path} nests a plugin component inside `.claude-plugin/`"
                );
                assert!(!path.is_empty(), "mcp={mcp}: emitted an empty path");
            }

            // Skill-count parity with the raw surface.
            assert_eq!(
                generate_plugin_skills(&opts).len(),
                generate_skills(&opts).len(),
                "mcp={mcp}: plugin/raw skill count parity"
            );
            assert_eq!(generate_plugin_skills(&opts).len(), 11, "mcp={mcp}");
        }
    }

    #[test]
    fn plugin_skill_table_covers_exactly_the_raw_skill_set() {
        let mut raw: Vec<&str> = generate_skills(&plugin_test_opts(false))
            .iter()
            .map(|s| s.relative_path.split('/').next().unwrap())
            .collect();
        let mut table: Vec<&str> = PLUGIN_SKILL_NAMES.iter().map(|(raw, _)| *raw).collect();
        assert_eq!(
            raw, table,
            "PLUGIN_SKILL_NAMES must list the raw skill directories in emission order"
        );
        raw.sort_unstable();
        raw.dedup();
        table.sort_unstable();
        table.dedup();
        assert_eq!(raw.len(), 11);
        assert_eq!(table.len(), 11);
    }

    #[test]
    fn plugin_skill_bodies_use_namespaced_workflow_refs() {
        let engine_stems: Vec<String> = generate_plugin_workflows()
            .iter()
            .map(|w| {
                w.relative_path
                    .trim_start_matches("workflows/")
                    .trim_end_matches(".js")
                    .to_string()
            })
            .collect();

        for mcp in [false, true] {
            for principles in [None, Some("PRINCIPLES.md".to_string())] {
                let opts = SkillOptions {
                    project: Some("demo".to_string()),
                    principles_file: principles.clone(),
                    mcp,
                };
                let skills = generate_plugin_skills(&opts);
                let joined: String = skills
                    .iter()
                    .map(|s| s.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");

                assert_eq!(
                    joined.matches(".claude/workflows/").count(),
                    0,
                    "mcp={mcp}: a plugin skill body still carries a `.claude/workflows/` path"
                );
                assert_eq!(
                    joined.matches("${CLAUDE_PLUGIN_ROOT}").count(),
                    0,
                    "mcp={mcp}: the scriptPath branch was not the Phase-1 decision"
                );
                assert_eq!(joined.matches("scriptPath").count(), 0, "mcp={mcp}");
                assert_eq!(
                    joined.matches("provisioned automatically by").count(),
                    0,
                    "mcp={mcp}: plugin shims are not provisioned by `agent-config --skills`"
                );

                // EVERY mention of a shipped engine is namespaced, not just
                // the ones that happened to sit inside a `.claude/workflows/`
                // path literal. A plugin-installed shim reaches its engine
                // only through the `rdm:` namespace, so a bare stem left in an
                // operative "invoke the Workflow with …" instruction would not
                // resolve at runtime.
                let plugin_tokens = rdm_tokens(&joined);
                for stem in &engine_stems {
                    let total = count_of(&plugin_tokens, stem.as_str());
                    let namespaced = joined.matches(&format!("{PLUGIN_NAME}:{stem}")).count();
                    assert_eq!(
                        total - namespaced,
                        0,
                        "mcp={mcp}: {total} mentions of {stem} but only {namespaced} namespaced — \
                         a bare engine reference survives in a plugin skill body"
                    );
                }

                // Non-vacuity: the shim engine really is referenced, and by
                // every mention the raw surface makes of it.
                let raw_joined: String = generate_skills(&opts)
                    .iter()
                    .map(|s| s.content.clone())
                    .collect::<Vec<_>>()
                    .join("\n");
                let raw_dispatch = count_of(&rdm_tokens(&raw_joined), "rdm-wf-dispatch-phase");
                assert!(raw_dispatch > 0, "mcp={mcp}: the check would be vacuous");
                assert_eq!(
                    joined.matches("rdm:rdm-wf-dispatch-phase").count(),
                    raw_dispatch,
                    "mcp={mcp}: expected all {raw_dispatch} raw engine mentions to be namespaced"
                );

                // Every namespaced reference the rewrite emits names a real
                // emitted engine — no `rdm:` prefix is ever attached to a
                // stem the plugin does not ship. Scoped to the invocation form
                // the rewrite produces; the templates also carry unrelated
                // pre-existing `<!-- rdm:review-spec:… -->` generator markers,
                // which are HTML comments rather than engine references.
                for (idx, _) in joined.match_indices("`rdm:") {
                    let rest = &joined[idx + "`rdm:".len()..];
                    let end = rest
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                        .unwrap_or(rest.len());
                    let named = &rest[..end];
                    assert!(
                        engine_stems.iter().any(|s| s == named),
                        "mcp={mcp}: `rdm:{named}` names no emitted engine"
                    );
                }
            }
        }
    }

    #[test]
    fn plugin_engine_refs_are_namespaced_by_token_delta() {
        // The engine-name counterpart of `plugin_skill_rename_is_total_by_token
        // _delta`: every raw mention of a SHIPPED engine stem must REAPPEAR
        // namespaced, and none may survive bare. Counting the delta against the
        // raw body is what makes this total rather than a spot check.
        let engine_stems: Vec<String> = generate_plugin_workflows()
            .iter()
            .map(|w| {
                w.relative_path
                    .trim_start_matches("workflows/")
                    .trim_end_matches(".js")
                    .to_string()
            })
            .collect();

        for mcp in [false, true] {
            let opts = plugin_test_opts(mcp);
            let raw_joined: String = generate_skills(&opts)
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            let plugin_joined: String = generate_plugin_skills(&opts)
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("\n");

            for stem in &engine_stems {
                let raw_total = count_of(&rdm_tokens(&raw_joined), stem.as_str());
                let plugin_total = count_of(&rdm_tokens(&plugin_joined), stem.as_str());
                let plugin_namespaced = plugin_joined
                    .matches(&format!("{PLUGIN_NAME}:{stem}"))
                    .count();
                assert_eq!(
                    plugin_total, raw_total,
                    "mcp={mcp}: plugin emission changed how often {stem} occurs"
                );
                assert_eq!(
                    plugin_namespaced, raw_total,
                    "mcp={mcp}: {plugin_namespaced} of {raw_total} {stem} mentions were namespaced"
                );
                assert_eq!(
                    raw_joined.matches(&format!("{PLUGIN_NAME}:{stem}")).count(),
                    0,
                    "mcp={mcp}: the RAW surface must stay un-namespaced"
                );
            }

            // The per-skill view: the three shim skills that actually dispatch
            // must each carry only namespaced references. `autopilot` is the
            // regression this pins — it never contained a `.claude/workflows/`
            // path literal, so a path-literal-only rewrite left all of its
            // mentions bare.
            for file in generate_plugin_skills(&opts) {
                for stem in &engine_stems {
                    let total = count_of(&rdm_tokens(&file.content), stem.as_str());
                    let namespaced = file
                        .content
                        .matches(&format!("{PLUGIN_NAME}:{stem}"))
                        .count();
                    assert_eq!(
                        total,
                        namespaced,
                        "mcp={mcp}: {} has {} bare mentions of {stem}",
                        file.relative_path,
                        total - namespaced
                    );
                }
            }
            let autopilot = generate_plugin_skills(&opts)
                .into_iter()
                .find(|f| f.relative_path == "skills/autopilot/SKILL.md")
                .expect("the autopilot shim is emitted");
            assert!(
                autopilot
                    .content
                    .matches("rdm:rdm-wf-dispatch-phase")
                    .count()
                    > 0,
                "mcp={mcp}: autopilot must namespace the engine it dispatches"
            );

            // Engines this distribution does NOT ship stay bare — namespacing
            // them would name a plugin entry that does not exist.
            assert_eq!(
                plugin_joined.matches("rdm:rdm-wf-estimate").count(),
                0,
                "mcp={mcp}: rdm-wf-estimate is not shipped and must not be namespaced"
            );
        }
    }

    #[test]
    fn rewrite_workflow_refs_is_total_over_both_raw_phrasings() {
        let with_clause = "Invoke the `rdm-wf-dispatch-phase` Workflow (`.claude/workflows/rdm-wf-dispatch-phase.js`, provisioned automatically by `rdm agent-config claude --skills`) via the tool.";
        let bare = "Invoke the `rdm-wf-dispatch-phase` Workflow (`.claude/workflows/rdm-wf-dispatch-phase.js`) via the tool.";
        for input in [with_clause, bare] {
            let out = rewrite_workflow_refs(input);
            assert_eq!(
                out.matches(".claude/workflows/").count(),
                0,
                "rewrite left a source-tree path behind: {out}"
            );
            // BOTH mentions — the prose one and the path one — are namespaced.
            assert_eq!(
                out.matches("rdm:rdm-wf-dispatch-phase").count(),
                2,
                "both engine mentions must be namespaced: {out}"
            );
            assert_eq!(
                count_of(&rdm_tokens(&out), "rdm-wf-dispatch-phase"),
                2,
                "the stem must survive inside the namespaced form: {out}"
            );
        }
        assert!(
            rewrite_workflow_refs(with_clause).contains(", installed by the `rdm` plugin"),
            "the false provisioning clause was not replaced"
        );
        // The other shipped engine is handled by the same table-driven loop.
        assert_eq!(
            rewrite_workflow_refs("see `.claude/workflows/rdm-wf-review-refute-fix.js`"),
            "see `rdm:rdm-wf-review-refute-fix`"
        );
        // A bare mention with no path literal anywhere — the shape the
        // autopilot shim uses at its single operative invocation step.
        assert_eq!(
            rewrite_workflow_refs("invoke the **`rdm-wf-dispatch-phase` Workflow** via the tool"),
            "invoke the **`rdm:rdm-wf-dispatch-phase` Workflow** via the tool"
        );
    }

    #[test]
    fn namespace_engine_refs_is_token_exact() {
        let stem = "rdm-wf-dispatch-phase";

        // Bare mentions, possessives, and file-name mentions all collapse to
        // the namespaced invocation form.
        assert_eq!(
            namespace_engine_refs("`rdm-wf-dispatch-phase`", stem),
            "`rdm:rdm-wf-dispatch-phase`"
        );
        assert_eq!(
            namespace_engine_refs("`rdm-wf-dispatch-phase`'s budgets", stem),
            "`rdm:rdm-wf-dispatch-phase`'s budgets"
        );
        assert_eq!(
            namespace_engine_refs("the pipeline `rdm-wf-dispatch-phase.js` embeds", stem),
            "the pipeline `rdm:rdm-wf-dispatch-phase` embeds"
        );

        // Idempotent: an already-namespaced reference is never double-prefixed.
        let once = namespace_engine_refs("`rdm-wf-dispatch-phase`", stem);
        assert_eq!(namespace_engine_refs(&once, stem), once);

        // Path-embedded and identifier-embedded occurrences are left alone.
        assert_eq!(
            namespace_engine_refs(".claude/workflows/rdm-wf-dispatch-phase.js", stem),
            ".claude/workflows/rdm-wf-dispatch-phase.js"
        );
        assert_eq!(
            namespace_engine_refs("x-rdm-wf-dispatch-phase", stem),
            "x-rdm-wf-dispatch-phase"
        );
        assert_eq!(
            namespace_engine_refs("rdm-wf-dispatch-phase-v2", stem),
            "rdm-wf-dispatch-phase-v2"
        );
        assert_eq!(
            namespace_engine_refs("rdm-wf-dispatch-phase.json", stem),
            "rdm-wf-dispatch-phase.json"
        );

        // A stem that is not this one is untouched.
        assert_eq!(
            namespace_engine_refs("`rdm-wf-estimate`", stem),
            "`rdm-wf-estimate`"
        );

        // Multi-byte content around a match survives.
        assert_eq!(
            namespace_engine_refs("— `rdm-wf-dispatch-phase` —", stem),
            "— `rdm:rdm-wf-dispatch-phase` —"
        );
    }

    #[test]
    fn append_plugin_rdm_bin_note_separates_the_note_in_both_newline_cases() {
        // No-op when the body never mentions `rdmBin`.
        assert_eq!(
            append_plugin_rdm_bin_note("no binary argument here\n".to_string()),
            "no binary argument here\n"
        );

        // Both branches of the trailing-newline guard must yield exactly one
        // blank line between the body's last line and the appended heading.
        for body in ["…pass `rdmBin`.\n", "…pass `rdmBin`."] {
            let out = append_plugin_rdm_bin_note(body.to_string());
            assert!(
                out.starts_with(body.trim_end_matches('\n')),
                "the original body was altered: {out}"
            );
            assert!(
                out.contains("…pass `rdmBin`.\n\n## Resolving `rdmBin` (plugin install)\n"),
                "expected exactly one blank line before the note: {out:?}"
            );
            assert_eq!(
                out.matches("## Resolving `rdmBin` (plugin install)")
                    .count(),
                1,
                "the note must be appended exactly once: {out:?}"
            );
        }
    }

    #[test]
    fn rewrite_skill_names_is_token_exact() {
        // `rdm-do` is a strict prefix of `rdm-document`: the maximal-token
        // scan must rename the whole token, never the prefix.
        assert_eq!(rewrite_skill_names("`rdm-document`"), "`document`");
        assert_eq!(rewrite_skill_names("`rdm-do`"), "`do`");
        assert_eq!(rewrite_skill_names("rdm-do/SKILL.md"), "do/SKILL.md");
        assert_eq!(
            rewrite_skill_names("name: rdm-plan-review"),
            "name: plan-review"
        );

        // Non-skill `rdm-*` identifiers survive verbatim.
        for preserved in [
            "rdm-wf-dispatch-phase",
            "rdm-wf-estimate",
            "rdm-mechanical",
            "rdm-next",
            "rdm-side",
            "rdm-review-on-finalize",
            "rdm-plan-review-on-create",
        ] {
            assert_eq!(
                rewrite_skill_names(preserved),
                preserved,
                "{preserved} must not be rewritten"
            );
        }

        // The preceding-byte guard.
        assert_eq!(rewrite_skill_names("--rdm-bin"), "--rdm-bin");
        assert_eq!(rewrite_skill_names("librdm-do"), "librdm-do");
        assert_eq!(rewrite_skill_names("x_rdm-land"), "x_rdm-land");

        // Trailing `-` is trimmed off the token, not swallowed.
        assert_eq!(rewrite_skill_names("rdm-land- "), "land- ");

        // Multi-byte content around a match survives.
        assert_eq!(
            rewrite_skill_names("— `rdm-land` — `rdm-autopilot` —"),
            "— `land` — `autopilot` —"
        );
    }

    #[test]
    fn plugin_rewrites_are_order_independent() {
        for mcp in [false, true] {
            for skill in generate_skills(&plugin_test_opts(mcp)) {
                let a = rewrite_skill_names(&rewrite_workflow_refs(&skill.content));
                let b = rewrite_workflow_refs(&rewrite_skill_names(&skill.content));
                assert_eq!(
                    a, b,
                    "mcp={mcp}: {} is order-sensitive",
                    skill.relative_path
                );
            }
        }
    }

    #[test]
    fn plugin_skill_bodies_preserve_non_skill_rdm_identifiers() {
        // CLI surface counts, then MCP. `rdm-side` appears only in the CLI
        // prose, so the pair is deliberately asymmetric.
        let expected: [(&str, usize, usize); 5] = [
            // Bumped 20/19 -> 22/21 when the `resumeFromRunId` recovery sections
            // began naming the real dispatch-phase file. Deliberate, as the
            // assertion message below instructs.
            ("rdm-wf-dispatch-phase", 22, 21),
            ("rdm-wf-estimate", 2, 2),
            ("rdm-mechanical", 1, 1),
            ("rdm-next", 1, 1),
            ("rdm-side", 1, 0),
        ];

        for mcp in [false, true] {
            let opts = plugin_test_opts(mcp);
            let raw_joined: String = generate_skills(&opts)
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            let plugin_joined: String = generate_plugin_skills(&opts)
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            let raw_counts = rdm_tokens(&raw_joined);
            let plugin_counts = rdm_tokens(&plugin_joined);

            for (token, cli_count, mcp_count) in expected {
                let want = if mcp { mcp_count } else { cli_count };
                assert_eq!(
                    count_of(&raw_counts, token),
                    want,
                    "mcp={mcp}: raw occurrence count for {token} moved — update the literal deliberately"
                );
                assert_eq!(
                    count_of(&plugin_counts, token),
                    want,
                    "mcp={mcp}: plugin emission changed the occurrence count of the non-skill identifier {token}"
                );
            }

            // And the rename is total in the other direction: not one of the
            // eleven raw skill names survives anywhere in a plugin body.
            for (raw_name, _) in PLUGIN_SKILL_NAMES {
                assert_eq!(
                    count_of(&plugin_counts, raw_name),
                    0,
                    "mcp={mcp}: {raw_name} survived the plugin rename"
                );
                assert!(
                    count_of(&raw_counts, raw_name) > 0,
                    "mcp={mcp}: {raw_name} is absent from raw emission — the check would be vacuous"
                );
            }
        }
    }

    #[test]
    fn plugin_skill_rename_is_total_by_token_delta() {
        // Every occurrence of a raw skill name must REAPPEAR as its plugin
        // name, not merely vanish. Counting the delta against the raw body
        // cancels prose words like "land" and "do" that legitimately occur in
        // both.
        for mcp in [false, true] {
            let opts = plugin_test_opts(mcp);
            let raw_joined: String = generate_skills(&opts)
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            let plugin_joined: String = generate_plugin_skills(&opts)
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            let raw_counts = kebab_tokens(&raw_joined);
            let plugin_counts = kebab_tokens(&plugin_joined);

            let mut renamed_total = 0usize;
            for (raw_name, plugin_name) in PLUGIN_SKILL_NAMES {
                let raw_old = count_of(&raw_counts, raw_name);
                assert!(
                    raw_old > 0,
                    "mcp={mcp}: {raw_name} absent from raw emission"
                );
                let delta = count_of(&plugin_counts, plugin_name) as i64
                    - count_of(&raw_counts, plugin_name) as i64;
                assert_eq!(
                    delta, raw_old as i64,
                    "mcp={mcp}: {raw_name} -> {plugin_name} gained {delta} occurrences but had {raw_old}"
                );
                renamed_total += raw_old;
            }
            assert_eq!(
                renamed_total, 46,
                "mcp={mcp}: expected 46 skill-name occurrences per surface"
            );

            // The directory name and the frontmatter `name:` line are covered
            // by the same pass.
            for file in generate_plugin_skills(&opts) {
                let dir = file
                    .relative_path
                    .trim_start_matches("skills/")
                    .trim_end_matches("/SKILL.md");
                assert!(
                    file.content.contains(&format!("\nname: {dir}\n")),
                    "mcp={mcp}: frontmatter name of {} does not match its directory",
                    file.relative_path
                );
            }
        }
    }

    #[test]
    fn plugin_skill_bodies_carry_the_rdm_bin_resolution_note() {
        for mcp in [false, true] {
            let opts = plugin_test_opts(mcp);
            let raw = generate_skills(&opts);
            let plugin = generate_plugin_skills(&opts);
            let mut noted = 0usize;
            for (raw_skill, plugin_skill) in raw.iter().zip(plugin.iter()) {
                let needs_note = raw_skill.content.contains("rdmBin");
                let has_note = plugin_skill
                    .content
                    .contains("## Resolving `rdmBin` (plugin install)");
                assert_eq!(
                    needs_note, has_note,
                    "mcp={mcp}: {} note presence does not match its rdmBin usage",
                    plugin_skill.relative_path
                );
                if has_note {
                    noted += 1;
                    assert!(
                        plugin_skill
                            .content
                            .contains("`RDM_BIN` environment variable")
                    );
                    assert!(plugin_skill.content.contains("--rdm-bin"));
                    assert!(plugin_skill.content.contains("rdm binary not found."));
                }
            }
            assert_eq!(
                noted, 3,
                "mcp={mcp}: expected the 3 rdmBin-carrying shims (autopilot, dispatch-phase, do)"
            );
        }
    }

    #[test]
    fn plugin_workflow_bytes_are_byte_identical_to_source() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("rdm-core manifest dir has a parent");
        let raw = generate_workflows();
        let plugin = generate_plugin_workflows();
        assert_eq!(plugin.len(), 2);
        assert_eq!(raw.len(), plugin.len());

        let expected_provenance_literals = [6usize, 3];
        let expected_meta_names = ["rdm-wf-dispatch-phase", "rdm-wf-review-refute-fix"];

        for (idx, (raw_wf, plugin_wf)) in raw.iter().zip(plugin.iter()).enumerate() {
            assert_eq!(
                plugin_wf.relative_path,
                format!("workflows/{}", raw_wf.relative_path)
            );
            assert_eq!(
                plugin_wf.content, raw_wf.content,
                "{} is not byte-identical to the raw emission",
                plugin_wf.relative_path
            );
            let source_path = repo_root
                .join(".claude/workflows")
                .join(raw_wf.relative_path);
            let source = std::fs::read_to_string(&source_path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", source_path.display()));
            assert_eq!(
                plugin_wf.content, source,
                "{} drifted from the dogfood copy",
                plugin_wf.relative_path
            );
            // The generator-provenance comments are byte-frozen; AC2's
            // ".claude/workflows/-free" rule is scoped to SKILL.md bodies.
            assert_eq!(
                plugin_wf.content.matches(".claude/workflows/").count(),
                expected_provenance_literals[idx],
                "{} lost or gained a provenance literal",
                plugin_wf.relative_path
            );
            // Transform 2 (the `meta.name` rewrite) was NOT selected.
            assert_eq!(
                parse_meta_name(&plugin_wf.content),
                expected_meta_names[idx],
                "{} had its meta.name rewritten",
                plugin_wf.relative_path
            );
        }
    }

    #[test]
    fn plugin_skill_and_engine_names_are_disjoint() {
        let skill_names: std::collections::BTreeSet<String> =
            generate_plugin_skills(&plugin_test_opts(false))
                .iter()
                .map(|f| {
                    f.relative_path
                        .trim_start_matches("skills/")
                        .trim_end_matches("/SKILL.md")
                        .to_string()
                })
                .collect();
        assert_eq!(skill_names.len(), 11);

        let engine_names: std::collections::BTreeSet<String> = generate_plugin_workflows()
            .iter()
            .map(|w| parse_meta_name(&w.content))
            .collect();
        // Non-vacuity: pin the engine names, which is what proves the
        // `rdm-wf-` disambiguator survived plugin emission.
        assert_eq!(
            engine_names,
            ["rdm-wf-dispatch-phase", "rdm-wf-review-refute-fix"]
                .into_iter()
                .map(String::from)
                .collect::<std::collections::BTreeSet<_>>()
        );

        assert!(
            skill_names.is_disjoint(&engine_names),
            "plugin skill names and engine meta.names collide: {:?}",
            skill_names.intersection(&engine_names).collect::<Vec<_>>()
        );

        // Planted collision: the check must bite.
        let mut colliding = engine_names.clone();
        colliding.insert("dispatch-phase".to_string());
        assert!(
            !skill_names.is_disjoint(&colliding),
            "the disjointness check does not detect a planted collision"
        );
    }

    #[test]
    fn plugin_manifest_round_trips_with_independent_version() {
        let manifest = generate_plugin_manifest();
        let parsed: serde_json::Value =
            serde_json::from_str(&manifest).expect("manifest is valid JSON");

        assert_eq!(parsed["name"], "rdm");
        assert!(
            parsed["description"]
                .as_str()
                .is_some_and(|d| !d.is_empty()),
            "manifest description must be a non-empty string"
        );
        assert!(
            parsed["author"]["name"]
                .as_str()
                .is_some_and(|n| !n.is_empty()),
            "manifest author.name must be a non-empty string"
        );
        assert!(
            parsed["author"]["url"]
                .as_str()
                .is_some_and(|u| !u.is_empty()),
            "manifest author.url must be a non-empty string"
        );
        assert!(
            parsed["workflows"].is_null(),
            "the `workflows` key must be absent — the directory is convention-discovered"
        );

        let version = parsed["version"]
            .as_str()
            .expect("manifest version is a JSON string");
        assert!(!version.is_empty(), "manifest version must be non-empty");
        let components: Vec<&str> = version.split('.').collect();
        assert_eq!(
            components.len(),
            3,
            "manifest version {version} is not a 3-component semver"
        );
        for component in &components {
            component
                .parse::<u64>()
                .unwrap_or_else(|e| panic!("semver component {component:?} of {version}: {e}"));
        }

        // Compared against a value read independently from the workspace
        // manifest, never against `env!("CARGO_PKG_VERSION")` — that would be
        // the tautology this test exists to avoid.
        let workspace_manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("rdm-core manifest dir has a parent")
            .join("Cargo.toml");
        let text = std::fs::read_to_string(&workspace_manifest)
            .unwrap_or_else(|e| panic!("read {}: {e}", workspace_manifest.display()));
        let workspace: toml::Value =
            toml::from_str(&text).expect("workspace Cargo.toml parses as TOML");
        let workspace_version = workspace["workspace"]["package"]["version"]
            .as_str()
            .expect("workspace.package.version is a string");
        assert_eq!(version, workspace_version);
    }

    /// Extracts the brace-balanced body of the function whose signature
    /// starts with `anchor`, panicking loudly (rather than passing
    /// vacuously) if the anchor cannot be located.
    fn fn_body(source: &str, anchor: &str) -> String {
        let start = source.find(anchor).unwrap_or_else(|| {
            panic!(
                "anchor {anchor:?} not found in agent_config.rs — this guard would pass vacuously"
            )
        });
        let open = start
            + source[start..]
                .find('{')
                .unwrap_or_else(|| panic!("no body brace after {anchor:?}"));
        let mut depth = 0usize;
        for (offset, ch) in source[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return source[open..open + offset + 1].to_string();
                    }
                }
                _ => {}
            }
        }
        panic!("unbalanced braces after {anchor:?}");
    }

    #[test]
    fn plugin_mode_adds_no_call_site_in_raw_emitters() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent_config.rs"),
        )
        .expect("read agent_config.rs");
        for anchor in ["fn generate_skills(", "fn render_skill("] {
            let body = fn_body(&source, anchor);
            assert!(
                !body.to_lowercase().contains("plugin"),
                "{anchor} body mentions plugin — plugin mode must stay a post-processing pass over the raw surface"
            );
            assert!(
                body.len() > 20,
                "{anchor} body extraction looks degenerate: {body:?}"
            );
        }
    }
}
