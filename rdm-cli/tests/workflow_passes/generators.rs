//! The shared generator gate for the three passes. Each pass's canonical
//! `lib/<pass>.mjs` block is stamped into `.claude/workflows/rdm-wf-<pass>.js`
//! by `scripts/gen-workflow-<pass>.sh`, which also syncs the engine's
//! embedded template copy (`rdm-core/src/templates/workflows/`) and its
//! plugin copy (`plugins/rdm/workflows/`). These helpers run the real
//! generator: `--check` on the real tree, and a drift → red → heal control in
//! a scratch copy, so the real tree is never edited.

use std::path::Path;
use std::process::{Command, Output};

use rdm_devtools::workflow::MutantTree;

use crate::workflow_support::repo_root;

/// One pass's generator and the files it reads and writes.
pub struct Generator {
    /// `scripts/gen-workflow-<pass>.sh`.
    pub script: &'static str,
    /// `.claude/workflows/lib/<pass>.mjs`.
    pub lib: &'static str,
    /// `.claude/workflows/rdm-wf-<pass>.js`.
    pub engine: &'static str,
    /// The embedded template copy.
    pub template: &'static str,
    /// The checked-in plugin copy.
    pub plugin: &'static str,
    /// The block's begin-marker text.
    pub begin: &'static str,
}

fn sh(root: &Path, script: &str, args: &[&str]) -> Output {
    Command::new("sh")
        .arg(root.join(script))
        .args(args)
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| panic!("running {script}: {e}"))
}

fn assert_ok(out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed ({:?}):\n{}{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

impl Generator {
    /// `--check` passes on the real tree.
    pub fn in_sync(&self) {
        assert_ok(
            &sh(&repo_root(), self.script, &["--check"]),
            &format!("{} --check on the real tree", self.script),
        );
    }

    /// In a scratch copy: a line planted inside the engine's stamped block,
    /// and separately a line appended to each full copy, each make `--check`
    /// fail; regenerating heals every file to its exact original bytes.
    pub fn drift_detected_then_healed(&self) {
        let tree = MutantTree::copy(
            &repo_root(),
            &[
                self.script,
                "scripts/lib/gen-workflow-block.sh",
                self.lib,
                self.engine,
                self.template,
                self.plugin,
            ],
        )
        .expect("copy scratch tree");
        let root = tree.root();
        assert_ok(
            &sh(root, self.script, &["--check"]),
            "scratch --check on a clean copy",
        );

        // Drift inside the stamped block of the engine.
        let engine = root.join(self.engine);
        let original = std::fs::read(&engine).expect("read engine");
        let text = String::from_utf8(original.clone()).expect("utf-8 engine");
        let at = text
            .find(self.begin)
            .unwrap_or_else(|| panic!("{} has no {:?} marker", self.engine, self.begin));
        let eol = at + text[at..].find('\n').expect("marker line ends");
        std::fs::write(
            &engine,
            format!("{}\nconst PLANTED_DRIFT = 1;{}", &text[..eol], &text[eol..]),
        )
        .expect("plant drift");
        self.red_then_healed(root, self.engine, &original, "block drift in the engine");

        // Drift in each full copy the generator syncs.
        for copy in [self.template, self.plugin] {
            let path = root.join(copy);
            let original = std::fs::read(&path).expect("read copy");
            let mut drifted = original.clone();
            drifted.extend_from_slice(b"\n// planted drift\n");
            std::fs::write(&path, drifted).expect("plant drift");
            self.red_then_healed(root, copy, &original, "drift in a synced copy");
        }
    }

    fn red_then_healed(&self, root: &Path, file: &str, original: &[u8], what: &str) {
        assert!(
            !sh(root, self.script, &["--check"]).status.success(),
            "--check must fail on {what} ({file})"
        );
        assert_ok(&sh(root, self.script, &[]), "regenerate");
        assert_ok(
            &sh(root, self.script, &["--check"]),
            "--check after regenerating",
        );
        assert_eq!(
            std::fs::read(root.join(file)).expect("read healed file"),
            original,
            "regeneration restores the exact bytes of {file}"
        );
    }
}
