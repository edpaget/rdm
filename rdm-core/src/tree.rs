/// Hierarchical tree view of plan repo contents.
///
/// Builds and formats a tree representation showing projects, roadmaps,
/// phases, and tasks with their statuses.
use serde::Serialize;

use crate::document::Document;
use crate::model::{Phase, Roadmap, Task};
use crate::store::Store;

/// A node in the plan hierarchy tree.
#[derive(Debug, Clone, Serialize)]
pub struct TreeNode {
    /// Display name (slug, stem, or project name).
    pub name: String,
    /// Kind of entity this node represents.
    pub kind: TreeNodeKind,
    /// Status string, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Child nodes.
    pub children: Vec<TreeNode>,
}

/// The kind of entity a tree node represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TreeNodeKind {
    /// A project.
    Project,
    /// A roadmap.
    Roadmap,
    /// A phase within a roadmap.
    Phase,
    /// A standalone task.
    Task,
}

/// Builds a tree for a single project.
///
/// # Errors
///
/// Returns an error if any repo operations fail (loading roadmaps, phases, or tasks).
pub fn build_tree(store: &impl Store, project: &str) -> crate::error::Result<TreeNode> {
    let mut children = Vec::new();

    let roadmaps = crate::ops::roadmap::list_roadmaps(store, project)?;
    for roadmap_doc in &roadmaps {
        let slug = &roadmap_doc.frontmatter.roadmap;
        let phases = crate::ops::phase::list_phases(store, project, slug)?;
        children.push(build_roadmap_node(roadmap_doc, &phases));
    }

    let tasks = crate::ops::task::list_tasks(store, project)?;
    for (slug, task_doc) in &tasks {
        children.push(build_task_node(slug, task_doc));
    }

    Ok(TreeNode {
        name: project.to_string(),
        kind: TreeNodeKind::Project,
        status: None,
        children,
    })
}

fn build_roadmap_node(doc: &Document<Roadmap>, phases: &[(String, Document<Phase>)]) -> TreeNode {
    let phase_children: Vec<TreeNode> = phases
        .iter()
        .map(|(stem, pd)| TreeNode {
            name: stem.clone(),
            kind: TreeNodeKind::Phase,
            status: Some(pd.frontmatter.status.to_string()),
            children: Vec::new(),
        })
        .collect();

    TreeNode {
        name: doc.frontmatter.roadmap.clone(),
        kind: TreeNodeKind::Roadmap,
        status: None,
        children: phase_children,
    }
}

fn build_task_node(slug: &str, doc: &Document<Task>) -> TreeNode {
    TreeNode {
        name: slug.to_string(),
        kind: TreeNodeKind::Task,
        status: Some(doc.frontmatter.status.to_string()),
        children: Vec::new(),
    }
}

/// Formats a tree as a human-readable string with Unicode box-drawing characters.
#[must_use]
pub fn format_tree(node: &TreeNode) -> String {
    let mut out = String::new();
    out.push_str(&format_root_line(node));
    out.push('\n');
    let len = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        let is_last = i == len - 1;
        format_subtree(child, "", is_last, &mut out);
    }
    out
}

fn format_root_line(node: &TreeNode) -> String {
    match &node.status {
        Some(s) => format!("{} ({})", node.name, s),
        None => node.name.clone(),
    }
}

fn format_subtree(node: &TreeNode, prefix: &str, is_last: bool, out: &mut String) {
    let connector = if is_last {
        "\u{2514}\u{2500}\u{2500} "
    } else {
        "\u{251c}\u{2500}\u{2500} "
    };
    let label = match &node.status {
        Some(s) => format!("{}{}{} [{}]", prefix, connector, node.name, s),
        None => format!("{}{}{}", prefix, connector, node.name),
    };
    out.push_str(&label);
    out.push('\n');

    let child_prefix = if is_last {
        format!("{prefix}    ")
    } else {
        format!("{prefix}\u{2502}   ")
    };
    let len = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        let child_is_last = i == len - 1;
        format_subtree(child, &child_prefix, child_is_last, out);
    }
}

/// Formats a tree as a Markdown indented list.
#[must_use]
pub fn format_tree_md(node: &TreeNode) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", node.name));
    for child in &node.children {
        format_tree_md_node(child, 0, &mut out);
    }
    out
}

fn format_tree_md_node(node: &TreeNode, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let label = match &node.status {
        Some(s) => format!("{indent}- **{}** `{s}`\n", node.name),
        None => format!("{indent}- **{}**\n", node.name),
    };
    out.push_str(&label);
    for child in &node.children {
        format_tree_md_node(child, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PhaseStatus, TaskStatus};

    fn make_tree() -> TreeNode {
        TreeNode {
            name: "myproject".to_string(),
            kind: TreeNodeKind::Project,
            status: None,
            children: vec![
                TreeNode {
                    name: "alpha".to_string(),
                    kind: TreeNodeKind::Roadmap,
                    status: None,
                    children: vec![
                        TreeNode {
                            name: "phase-1-setup".to_string(),
                            kind: TreeNodeKind::Phase,
                            status: Some(PhaseStatus::Done.to_string()),
                            children: Vec::new(),
                        },
                        TreeNode {
                            name: "phase-2-impl".to_string(),
                            kind: TreeNodeKind::Phase,
                            status: Some(PhaseStatus::InProgress.to_string()),
                            children: Vec::new(),
                        },
                    ],
                },
                TreeNode {
                    name: "fix-bug".to_string(),
                    kind: TreeNodeKind::Task,
                    status: Some(TaskStatus::Open.to_string()),
                    children: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn tree_human_format_contains_project_and_children() {
        let tree = make_tree();
        let output = format_tree(&tree);
        assert!(output.contains("myproject"));
        assert!(output.contains("alpha"));
        assert!(output.contains("phase-1-setup"));
        assert!(output.contains("[done]"));
        assert!(output.contains("[in-progress]"));
        assert!(output.contains("fix-bug"));
        assert!(output.contains("[open]"));
    }

    #[test]
    fn tree_human_format_uses_box_drawing() {
        let tree = make_tree();
        let output = format_tree(&tree);
        assert!(
            output.contains("\u{251c}\u{2500}\u{2500}")
                || output.contains("\u{2514}\u{2500}\u{2500}")
        );
    }

    #[test]
    fn tree_md_format_contains_headings_and_items() {
        let tree = make_tree();
        let output = format_tree_md(&tree);
        assert!(output.contains("# myproject"));
        assert!(output.contains("- **alpha**"));
        assert!(output.contains("- **phase-1-setup** `done`"));
        assert!(output.contains("- **fix-bug** `open`"));
    }

    #[test]
    fn tree_md_format_indents_children() {
        let tree = make_tree();
        let output = format_tree_md(&tree);
        // Phases should be indented under roadmap
        assert!(output.contains("  - **phase-1-setup**"));
    }

    #[test]
    fn tree_json_serializes() {
        let tree = make_tree();
        let json = serde_json::to_string_pretty(&tree).unwrap();
        assert!(json.contains("\"myproject\""));
        assert!(json.contains("\"project\""));
        assert!(json.contains("\"roadmap\""));
        assert!(json.contains("\"phase\""));
        assert!(json.contains("\"task\""));
    }

    #[test]
    fn tree_empty_project() {
        let tree = TreeNode {
            name: "empty".to_string(),
            kind: TreeNodeKind::Project,
            status: None,
            children: Vec::new(),
        };
        let output = format_tree(&tree);
        assert_eq!(output, "empty\n");
    }
}
