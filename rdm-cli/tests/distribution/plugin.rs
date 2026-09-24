//! The Claude Code plugin channel: `rdm agent-config claude --plugin`'s
//! emitted layout and manifest, its naming relation to the `--skills`
//! emission, its rejection messages, the checked-in `plugins/rdm/` tree kept
//! equal to fresh generator output (manifest version normalized on both
//! sides), and `.claude-plugin/marketplace.json`'s shape and sources.
//!
//! Every emission runs in a sandbox with no inherited `RDM_*` and no
//! `--project`, into a temp tree. The flag-conflict rejections themselves are
//! `cli_agent_config.rs`'s `agent_config_plugin_{and_skills_conflict,
//! requires_out,and_user_rejected_with_distinct_message,rejected_on_*}` and
//! `agent_config_skills_and_user_still_works_unaffected_by_plugin`; the live
//! `claude plugin install` observation stays in
//! `scripts/observe-plugin-install.sh`.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Value, json};

use crate::support::{
    Emit, REGENERATE_PLUGIN, entries, marketplace_problems, normalized_plugin_tree, repo_root,
    skill_problems, tree_diff,
};

fn manifest(root: &Path) -> Value {
    let bytes = std::fs::read(root.join(".claude-plugin/plugin.json")).expect("read manifest");
    serde_json::from_slice(&bytes).expect("manifest is JSON")
}

fn non_empty(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.trim().is_empty())
}

#[test]
fn emitted_plugin_layout_and_manifest() {
    let emit = Emit::new();
    let (root, _) = emit.plugin("plugin");
    assert_eq!(
        entries(&root),
        BTreeSet::from([
            ".claude-plugin".to_owned(),
            "agents".to_owned(),
            "skills".to_owned(),
            "workflows".to_owned()
        ])
    );
    assert_eq!(
        entries(&root.join(".claude-plugin")),
        BTreeSet::from(["plugin.json".to_owned()]),
        ".claude-plugin/ holds only the manifest"
    );
    let m = manifest(&root);
    assert_eq!(m["name"], "rdm");
    for key in ["version", "description"] {
        assert!(non_empty(&m[key]), "manifest {key} is set: {m}");
    }
    assert!(
        non_empty(&m["author"]["name"]) && non_empty(&m["author"]["url"]),
        "{m}"
    );
    assert!(
        m.get("workflows").is_none(),
        "the manifest declares no workflows key: {m}"
    );
}

#[test]
fn naming_transform_relates_to_the_skills_emission() {
    let emit = Emit::new();
    let (skills_root, _) = emit.skills("skills");
    let (plugin_root, _) = emit.plugin("plugin");
    let stripped: BTreeSet<String> = entries(&skills_root.join(".claude/skills"))
        .into_iter()
        .map(|d| {
            d.strip_prefix("rdm-")
                .unwrap_or_else(|| panic!("a --skills directory without the rdm- prefix: {d}"))
                .to_owned()
        })
        .collect();
    assert_eq!(
        entries(&plugin_root.join("skills")),
        stripped,
        "plugin skill directories drop the rdm- prefix"
    );
    assert_eq!(
        entries(&plugin_root.join("workflows")),
        entries(&skills_root.join(".claude/workflows")),
        "plugin engines keep their names"
    );
}

#[test]
fn skill_frontmatter_names_match_their_dirs() {
    let emit = Emit::new();
    let (root, _) = emit.plugin("plugin");
    let dirs = entries(&root.join("skills"));
    assert!(!dirs.is_empty());
    let problems = skill_problems(&root.join("skills"), &dirs);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn rejection_messages_are_pairwise_distinct() {
    let emit = Emit::new();
    let stderr_of = |args: &[&str]| {
        let out = emit.rdm(args);
        assert!(
            !out.status.success(),
            "`rdm {}` is rejected",
            args.join(" ")
        );
        String::from_utf8_lossy(&out.stderr).trim().to_owned()
    };
    let no_dest = stderr_of(&["agent-config", "claude", "--plugin"]);
    let user = stderr_of(&["agent-config", "claude", "--plugin", "--user"]);
    let out_dir = emit.path("neg-platform");
    let platform = stderr_of(&[
        "agent-config",
        "pi",
        "--plugin",
        "--out",
        &out_dir.to_string_lossy(),
    ]);
    let skills_dest = stderr_of(&["agent-config", "claude", "--skills"]);
    for msg in [&no_dest, &user, &platform, &skills_dest] {
        assert!(!msg.is_empty(), "every rejection explains itself");
    }
    assert_ne!(no_dest, user);
    assert_ne!(no_dest, platform);
    assert_ne!(user, platform);
    for msg in [&no_dest, &user, &platform] {
        assert!(
            !msg.contains(skills_dest.as_str()),
            "a --plugin rejection reuses the --skills destination error: {msg}"
        );
    }
    assert!(!out_dir.exists(), "a rejected emission writes nothing");
    assert!(
        !emit.sandbox.home.join(".claude").exists(),
        "a rejected --user emission writes nothing"
    );
}

#[test]
fn checked_in_tree_matches_the_generator() {
    let emit = Emit::new();
    let (fresh, _) = emit.plugin("fresh");
    let committed =
        normalized_plugin_tree(&repo_root().join("plugins/rdm")).unwrap_or_else(|e| panic!("{e}"));
    let generated = normalized_plugin_tree(&fresh).unwrap_or_else(|e| panic!("{e}"));
    let drift = tree_diff(&committed, &generated);
    assert!(
        drift.is_empty(),
        "plugins/rdm/ has drifted from the generator (first tree: checked in; second: fresh):\n  {}\nRegenerate with:\n  {REGENERATE_PLUGIN}",
        drift.join("\n  ")
    );
}

#[test]
fn fresh_manifest_version_is_the_crate_version() {
    let emit = Emit::new();
    let (fresh, _) = emit.plugin("fresh");
    assert_eq!(manifest(&fresh)["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn marketplace_shape_and_sources_resolve() {
    let root = repo_root();
    let market: Value = serde_json::from_slice(
        &std::fs::read(root.join(".claude-plugin/marketplace.json")).expect("read marketplace"),
    )
    .expect("marketplace is JSON");
    let problems = marketplace_problems(&market, &root);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn marketplace_checker_rejects_planted_corruptions() {
    let root = repo_root();
    let good: Value = serde_json::from_slice(
        &std::fs::read(root.join(".claude-plugin/marketplace.json")).expect("read marketplace"),
    )
    .expect("marketplace is JSON");
    assert!(marketplace_problems(&good, &root).is_empty());
    fn remove(m: &mut Value, key: &str) {
        if let Some(o) = m.as_object_mut() {
            o.remove(key);
        }
    }
    type Corruption = fn(&mut Value);
    let cases: [(&str, Corruption); 9] = [
        ("dangling source", |m| {
            m["plugins"][0]["source"] = json!("./plugins/does-not-exist");
        }),
        ("empty plugin list", |m| m["plugins"] = json!([])),
        ("missing plugin list", |m| remove(m, "plugins")),
        ("entry name disagrees with the manifest", |m| {
            m["plugins"][0]["name"] = json!("not-rdm");
        }),
        ("empty name", |m| m["name"] = json!("")),
        ("missing description", |m| remove(m, "description")),
        ("blank owner name", |m| m["owner"]["name"] = json!("  ")),
        ("missing owner url", |m| remove(&mut m["owner"], "url")),
        ("empty source", |m| m["plugins"][0]["source"] = json!("")),
    ];
    for (name, corrupt) in cases {
        let mut m = good.clone();
        corrupt(&mut m);
        assert!(
            !marketplace_problems(&m, &root).is_empty(),
            "the checker misses: {name}"
        );
    }
}

#[test]
fn drift_normalization_is_surgical() {
    let emit = Emit::new();
    let (fresh, _) = emit.plugin("fresh");
    let base = normalized_plugin_tree(&fresh).unwrap_or_else(|e| panic!("{e}"));
    let copies = std::cell::Cell::new(0);
    let edited = |rel: &str, f: &dyn Fn(Vec<u8>) -> Vec<u8>| {
        copies.set(copies.get() + 1);
        let (copy, _) = emit.plugin(&format!("copy-{}", copies.get()));
        let path = copy.join(rel);
        let bytes = std::fs::read(&path).expect("read the file to edit");
        std::fs::write(&path, f(bytes)).expect("edit the copy");
        normalized_plugin_tree(&copy).map(|t| tree_diff(&base, &t))
    };
    // A version-only change is normalized away.
    let bumped = edited(".claude-plugin/plugin.json", &|b| {
        let m: Value = serde_json::from_slice(&b).expect("manifest");
        let old = serde_json::to_string(&m["version"]).expect("version");
        String::from_utf8(b)
            .expect("utf8")
            .replacen(&old, "\"99.99.99\"", 1)
            .into_bytes()
    });
    assert_eq!(bumped, Ok(Vec::new()), "a version bump alone is not drift");
    // Any other manifest field is.
    let described = edited(".claude-plugin/plugin.json", &|b| {
        String::from_utf8(b)
            .expect("utf8")
            .replacen("\"description\": \"", "\"description\": \"X", 1)
            .into_bytes()
    });
    assert!(matches!(described, Ok(d) if !d.is_empty()));
    // So is one byte of a skill body.
    let skill = entries(&fresh.join("skills"))
        .into_iter()
        .next()
        .expect("a skill");
    let body = edited(&format!("skills/{skill}/SKILL.md"), &|mut b| {
        b.push(b'\n');
        b
    });
    assert!(matches!(body, Ok(d) if !d.is_empty()));
    // So is a stray file.
    let (copy, _) = emit.plugin("stray");
    std::fs::write(copy.join("workflows/stray.js"), "stray\n").expect("plant a stray file");
    let stray = tree_diff(&base, &normalized_plugin_tree(&copy).expect("normalize"));
    assert_eq!(
        stray,
        vec!["only in the second tree: workflows/stray.js".to_owned()]
    );
}
