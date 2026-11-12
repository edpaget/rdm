//! Back-compat for what the plan-review tag gate left behind: a config file
//! that still sets the retired `plan_review` key, an environment that still
//! exports `RDM_PLAN_REVIEW`, a caller that still passes `task create
//! --no-plan-review`, and items that still carry a `needs-plan-review` tag.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// Where a fixture still sets the retired `plan_review` key.
#[derive(Clone, Copy, Debug, Default)]
struct Legacy {
    /// `plan_review = true` at the top level of `rdm.toml`.
    repo: bool,
    /// `plan_review = true` under `[projects.acme]` in `rdm.toml`.
    project: bool,
    /// `plan_review = true` in the global config file.
    global: bool,
    /// The value of `RDM_PLAN_REVIEW` every command runs with.
    env: Option<&'static str>,
}

const REPO_KEY: Legacy = Legacy {
    repo: true,
    project: false,
    global: false,
    env: None,
};

/// Every way a setup that enabled the tag gate can still look.
const LEGACY_SETUPS: [Legacy; 6] = [
    REPO_KEY,
    Legacy {
        repo: false,
        project: true,
        global: false,
        env: None,
    },
    Legacy {
        repo: false,
        project: false,
        global: true,
        env: None,
    },
    Legacy {
        repo: false,
        project: false,
        global: false,
        env: Some("true"),
    },
    // A value the variable used to reject, on top of a file that sets the key.
    Legacy {
        repo: true,
        project: false,
        global: false,
        env: Some("yes"),
    },
    // The empty value was rejected too.
    Legacy {
        repo: true,
        project: true,
        global: true,
        env: Some(""),
    },
];

/// A plan repo with project `acme` and its own global config directory.
struct Fixture {
    root: TempDir,
    config: TempDir,
    env: Option<&'static str>,
}

impl Fixture {
    fn new(legacy: Legacy) -> Self {
        let fixture = Fixture {
            root: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            env: legacy.env,
        };
        fixture.run(&["init"]);
        fixture.run(&["project", "create", "acme"]);

        // The key is written into the files by hand: `rdm config set` refuses it.
        let mut repo_toml = String::new();
        if legacy.repo {
            repo_toml.push_str("plan_review = true\n");
        }
        if legacy.project {
            repo_toml.push_str("\n[projects.acme]\nplan_review = true\n");
        }
        std::fs::write(fixture.repo_config_path(), repo_toml).unwrap();
        if legacy.global {
            std::fs::create_dir_all(fixture.config.path().join("rdm")).unwrap();
            std::fs::write(fixture.global_config_path(), "plan_review = true\n").unwrap();
        }
        fixture
    }

    fn repo_config_path(&self) -> std::path::PathBuf {
        self.root.path().join("rdm.toml")
    }

    fn global_config_path(&self) -> std::path::PathBuf {
        self.config.path().join("rdm").join("config.toml")
    }

    /// The bytes of both config files, `None` for one that does not exist.
    fn config_bytes(&self) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        (
            std::fs::read(self.repo_config_path()).ok(),
            std::fs::read(self.global_config_path()).ok(),
        )
    }

    fn rdm(&self) -> Command {
        let mut cmd = Command::cargo_bin("rdm").unwrap();
        cmd.env("XDG_CONFIG_HOME", self.config.path());
        for var in ["RDM_ROOT", "RDM_PROJECT", "RDM_FORMAT", "RDM_PLAN_REVIEW"] {
            cmd.env_remove(var);
        }
        if let Some(value) = self.env {
            cmd.env("RDM_PLAN_REVIEW", value);
        }
        cmd.arg("--root").arg(self.root.path());
        cmd
    }

    /// Runs a command that must succeed without mentioning the retired key
    /// or variable on stderr, and returns its stdout.
    fn run(&self, args: &[&str]) -> String {
        let output = self.rdm().args(args).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "`rdm {}` failed under {:?}: {stderr}",
            args.join(" "),
            self.env
        );
        assert!(
            !stderr.contains("plan_review") && !stderr.contains("PLAN_REVIEW"),
            "`rdm {}` mentioned the retired key on stderr: {stderr}",
            args.join(" ")
        );
        String::from_utf8(output.stdout).unwrap()
    }

    /// Runs a command that must fail, and returns its stderr.
    fn refused(&self, args: &[&str]) -> String {
        let output = self.rdm().args(args).output().unwrap();
        assert!(
            !output.status.success(),
            "`rdm {}` unexpectedly succeeded",
            args.join(" ")
        );
        String::from_utf8(output.stderr).unwrap()
    }

    /// The `tags` of the item `show_args` shows, empty when it has none.
    fn tags(&self, show_args: &[&str]) -> Vec<String> {
        let mut args = show_args.to_vec();
        args.extend(["--project", "acme", "--format", "json"]);
        let shown: Value = serde_json::from_str(&self.run(&args)).unwrap();
        shown["tags"]
            .as_array()
            .map(|tags| {
                tags.iter()
                    .map(|tag| tag.as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Creates roadmap `<slug>`, its phase 1 and task `<slug>`, each with
    /// `extra` appended to the create command.
    fn create_each_kind(&self, slug: &str, extra: &[&str]) {
        let with_extra = |base: &[&str]| {
            let mut args = base.to_vec();
            args.extend(["--project", "acme", "--no-edit"]);
            args.extend(extra);
            self.run(&args);
        };
        with_extra(&["roadmap", "create", slug, "--title", "A roadmap"]);
        with_extra(&[
            "phase",
            "create",
            "build",
            "--title",
            "A phase",
            "--number",
            "1",
            "--roadmap",
            slug,
        ]);
        with_extra(&["task", "create", slug, "--title", "A task"]);
    }

    /// The tags of roadmap `<slug>`, its phase 1 and task `<slug>`.
    fn tags_of_each_kind(&self, slug: &str) -> [Vec<String>; 3] {
        [
            self.tags(&["roadmap", "show", slug]),
            self.tags(&["phase", "show", "1", "--roadmap", slug]),
            self.tags(&["task", "show", slug]),
        ]
    }

    /// The identifiers `rdm search "" --tag <tag>` returns, sorted.
    fn tagged(&self, tag: &str) -> Vec<String> {
        let found: Value = serde_json::from_str(&self.run(&[
            "search",
            "",
            "--tag",
            tag,
            "--project",
            "acme",
            "--format",
            "json",
        ]))
        .unwrap();
        let mut identifiers: Vec<String> = found
            .as_array()
            .unwrap()
            .iter()
            .map(|hit| {
                format!(
                    "{}:{}",
                    hit["kind"].as_str().unwrap(),
                    hit["identifier"].as_str().unwrap()
                )
            })
            .collect();
        identifiers.sort();
        identifiers
    }
}

fn strs(tags: &[&str]) -> Vec<String> {
    tags.iter().map(|tag| tag.to_string()).collect()
}

#[test]
fn create_writes_exactly_the_tags_passed_wherever_plan_review_is_still_set() {
    for legacy in LEGACY_SETUPS {
        let fixture = Fixture::new(legacy);
        fixture.create_each_kind("tagged", &["--tags", "a,b"]);
        fixture.create_each_kind("bare", &[]);

        for tags in fixture.tags_of_each_kind("tagged") {
            assert_eq!(tags, strs(&["a", "b"]), "{legacy:?}");
        }
        for tags in fixture.tags_of_each_kind("bare") {
            assert_eq!(tags, strs(&[]), "{legacy:?}");
        }
    }
}

#[test]
fn every_command_family_runs_against_a_config_that_still_sets_plan_review() {
    for legacy in LEGACY_SETUPS {
        let fixture = Fixture::new(legacy);
        fixture.create_each_kind("work", &["--tags", "a"]);

        fixture.run(&["roadmap", "list", "--project", "acme"]);
        fixture.run(&["roadmap", "show", "work", "--project", "acme"]);
        fixture.run(&["phase", "list", "--roadmap", "work", "--project", "acme"]);
        fixture.run(&["task", "list", "--project", "acme"]);
        fixture.run(&["task", "show", "work", "--project", "acme"]);
        fixture.run(&[
            "phase",
            "update",
            "1",
            "--status",
            "in-progress",
            "--roadmap",
            "work",
            "--project",
            "acme",
            "--no-edit",
        ]);
        fixture.run(&[
            "task",
            "update",
            "work",
            "--status",
            "in-progress",
            "--project",
            "acme",
            "--no-edit",
        ]);
        fixture.run(&["search", "roadmap", "--project", "acme"]);
        fixture.run(&["status"]);

        let listed = fixture.run(&["config", "list"]);
        assert!(
            !listed.contains("plan_review"),
            "config list still has a plan_review row under {legacy:?}: {listed}"
        );
        let scoped = fixture.run(&["config", "list", "--project", "acme"]);
        assert_eq!(scoped.lines().count(), 4, "{legacy:?}: {scoped}");

        fixture.run(&["config", "get", "default_branch"]);
        fixture.run(&["config", "set", "default_branch", "trunk"]);
        let branch = fixture.run(&["config", "get", "default_branch", "--raw"]);
        assert_eq!(branch.trim(), "trunk", "{legacy:?}");
        fixture.run(&["roadmap", "list", "--project", "acme"]);
        fixture.run(&["commit", "-m", "plan: land the fixture"]);
    }
}

#[test]
fn config_get_and_set_refuse_the_retired_plan_review_key_and_write_nothing() {
    let sets_the_key = Legacy {
        repo: true,
        project: true,
        global: true,
        env: None,
    };
    for legacy in [sets_the_key, Legacy::default()] {
        let fixture = Fixture::new(legacy);
        let before = fixture.config_bytes();

        for args in [
            &["config", "get", "plan_review"][..],
            &["config", "get", "plan_review", "--project", "acme"],
            &["config", "set", "plan_review", "true"],
            &["config", "set", "plan_review", "true", "--global"],
            &["config", "set", "plan_review", "true", "--project", "acme"],
        ] {
            let stderr = fixture.refused(args);
            assert!(
                stderr.contains("plan_review") && stderr.contains("retired"),
                "`rdm {}` under {legacy:?}: {stderr}",
                args.join(" ")
            );
            assert_eq!(fixture.config_bytes(), before, "`rdm {}`", args.join(" "));
        }
    }
}

#[test]
fn task_create_no_plan_review_is_accepted_and_changes_nothing() {
    for legacy in [REPO_KEY, Legacy::default()] {
        let fixture = Fixture::new(legacy);
        fixture.run(&[
            "task",
            "create",
            "finding",
            "--title",
            "A finding",
            "--tags",
            "x",
            "--no-plan-review",
            "--project",
            "acme",
            "--no-edit",
        ]);
        assert_eq!(
            fixture.tags(&["task", "show", "finding"]),
            strs(&["x"]),
            "{legacy:?}"
        );
    }
}

#[test]
fn an_item_already_tagged_needs_plan_review_shows_keeps_and_loses_it_like_any_tag() {
    let fixture = Fixture::new(REPO_KEY);
    fixture.create_each_kind("old", &["--tags", "needs-plan-review,keep"]);
    let every_kind = strs(&["phase:old/phase-1-build", "roadmap:old", "task:old"]);

    // It shows.
    for tags in fixture.tags_of_each_kind("old") {
        assert_eq!(tags, strs(&["needs-plan-review", "keep"]));
    }
    assert_eq!(fixture.tagged("needs-plan-review"), every_kind);

    // An update that does not touch tags keeps it.
    fixture.run(&[
        "roadmap",
        "update",
        "old",
        "--body",
        "Reworded.",
        "--project",
        "acme",
        "--no-edit",
    ]);
    fixture.run(&[
        "phase",
        "update",
        "1",
        "--status",
        "in-progress",
        "--body",
        "Reworded.",
        "--roadmap",
        "old",
        "--project",
        "acme",
        "--no-edit",
    ]);
    fixture.run(&[
        "task",
        "update",
        "old",
        "--status",
        "in-progress",
        "--body",
        "Reworded.",
        "--project",
        "acme",
        "--no-edit",
    ]);
    for tags in fixture.tags_of_each_kind("old") {
        assert_eq!(tags, strs(&["needs-plan-review", "keep"]));
    }

    // So does adding another tag.
    let update_each = |flag: &str, value: &str| {
        fixture.run(&[
            "roadmap",
            "update",
            "old",
            flag,
            value,
            "--project",
            "acme",
            "--no-edit",
        ]);
        fixture.run(&[
            "phase",
            "update",
            "1",
            flag,
            value,
            "--roadmap",
            "old",
            "--project",
            "acme",
            "--no-edit",
        ]);
        fixture.run(&[
            "task",
            "update",
            "old",
            flag,
            value,
            "--project",
            "acme",
            "--no-edit",
        ]);
    };
    update_each("--add-tag", "other");
    for tags in fixture.tags_of_each_kind("old") {
        assert_eq!(tags, strs(&["needs-plan-review", "keep", "other"]));
    }
    assert_eq!(fixture.tagged("needs-plan-review"), every_kind);

    // An ordinary `--tags` write that omits it removes it.
    update_each("--tags", "keep");
    for tags in fixture.tags_of_each_kind("old") {
        assert_eq!(tags, strs(&["keep"]));
    }
    assert_eq!(fixture.tagged("needs-plan-review"), strs(&[]));
    assert_eq!(fixture.tagged("keep"), every_kind);
}
