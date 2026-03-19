/// Model introspection: discover what rdm tracks and the shape of each entity.
///
/// The [`Describe`] trait and supporting types let agents and tools inspect
/// rdm's data model at runtime without reading source code.
use serde::Serialize;

/// Metadata about a single field on an entity.
#[derive(Debug, Clone, Serialize)]
pub struct FieldInfo {
    /// Field name (matches the serde/YAML key).
    pub name: &'static str,
    /// Human-readable type description (e.g. "string", "date (YYYY-MM-DD)").
    pub type_name: &'static str,
    /// Whether the field is always present (not optional).
    pub required: bool,
    /// Allowed values for enum fields (empty slice for free-form fields).
    pub enum_values: &'static [&'static str],
    /// Short description of what the field represents.
    pub description: &'static str,
}

/// Metadata about an entity type (project, roadmap, phase, or task).
#[derive(Debug, Clone, Serialize)]
pub struct EntityInfo {
    /// Entity name (lowercase, e.g. "project").
    pub name: &'static str,
    /// One-line description of this entity.
    pub description: &'static str,
    /// Fields in the entity's frontmatter.
    pub fields: Vec<FieldInfo>,
}

/// Trait for types that can describe their schema.
pub trait Describe {
    /// Returns metadata about this entity's fields and structure.
    fn describe() -> EntityInfo;
}

impl Describe for crate::model::Project {
    fn describe() -> EntityInfo {
        EntityInfo {
            name: "project",
            description: "A project directory grouping roadmaps and tasks.",
            fields: vec![
                FieldInfo {
                    name: "name",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Project slug identifier (used in directory names and references).",
                },
                FieldInfo {
                    name: "title",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Human-readable title.",
                },
            ],
        }
    }
}

impl Describe for crate::model::Roadmap {
    fn describe() -> EntityInfo {
        EntityInfo {
            name: "roadmap",
            description: "An ordered sequence of phases tracking a multi-step initiative.",
            fields: vec![
                FieldInfo {
                    name: "project",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Project this roadmap belongs to.",
                },
                FieldInfo {
                    name: "roadmap",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Roadmap slug identifier.",
                },
                FieldInfo {
                    name: "title",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Human-readable title.",
                },
                FieldInfo {
                    name: "phases",
                    type_name: "list of strings",
                    required: true,
                    enum_values: &[],
                    description: "Ordered list of phase file stems.",
                },
                FieldInfo {
                    name: "dependencies",
                    type_name: "list of strings",
                    required: false,
                    enum_values: &[],
                    description: "Roadmap slugs that must complete before this one.",
                },
            ],
        }
    }
}

impl Describe for crate::model::Phase {
    fn describe() -> EntityInfo {
        EntityInfo {
            name: "phase",
            description: "A single step within a roadmap, tracked by status.",
            fields: vec![
                FieldInfo {
                    name: "phase",
                    type_name: "integer",
                    required: true,
                    enum_values: &[],
                    description: "Phase number (1-based ordering).",
                },
                FieldInfo {
                    name: "title",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Human-readable title.",
                },
                FieldInfo {
                    name: "status",
                    type_name: "enum",
                    required: true,
                    enum_values: &["not-started", "in-progress", "done", "blocked"],
                    description: "Current status.",
                },
                FieldInfo {
                    name: "completed",
                    type_name: "date (YYYY-MM-DD)",
                    required: false,
                    enum_values: &[],
                    description: "Date the phase was completed, if applicable.",
                },
            ],
        }
    }
}

impl Describe for crate::model::Task {
    fn describe() -> EntityInfo {
        EntityInfo {
            name: "task",
            description: "A standalone work item tracked by status and priority.",
            fields: vec![
                FieldInfo {
                    name: "project",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Project this task belongs to.",
                },
                FieldInfo {
                    name: "title",
                    type_name: "string",
                    required: true,
                    enum_values: &[],
                    description: "Human-readable title.",
                },
                FieldInfo {
                    name: "status",
                    type_name: "enum",
                    required: true,
                    enum_values: &["open", "in-progress", "done", "wont-fix"],
                    description: "Current status.",
                },
                FieldInfo {
                    name: "priority",
                    type_name: "enum",
                    required: true,
                    enum_values: &["low", "medium", "high", "critical"],
                    description: "Priority level.",
                },
                FieldInfo {
                    name: "created",
                    type_name: "date (YYYY-MM-DD)",
                    required: true,
                    enum_values: &[],
                    description: "Date the task was created.",
                },
                FieldInfo {
                    name: "tags",
                    type_name: "list of strings",
                    required: false,
                    enum_values: &[],
                    description: "Optional tags for categorization.",
                },
            ],
        }
    }
}

/// Returns entity descriptions for all model types.
#[must_use]
pub fn all_entities() -> Vec<EntityInfo> {
    use crate::model::{Phase, Project, Roadmap, Task};
    vec![
        Project::describe(),
        Roadmap::describe(),
        Phase::describe(),
        Task::describe(),
    ]
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// Formats all entities as a human-readable list.
#[must_use]
pub fn format_entity_list(entities: &[EntityInfo]) -> String {
    let mut out = String::new();
    out.push_str("Entity types:\n\n");
    for e in entities {
        out.push_str(&format!("  {} — {}\n", e.name, e.description));
    }
    out.push_str("\nUse `rdm describe <entity>` to see fields.\n");
    out
}

/// Formats a single entity as a human-readable field table.
#[must_use]
pub fn format_entity_detail(entity: &EntityInfo) -> String {
    let mut out = String::new();
    out.push_str(&format!("{} — {}\n\n", entity.name, entity.description));
    out.push_str("Fields:\n\n");
    for f in &entity.fields {
        let req = if f.required { "required" } else { "optional" };
        out.push_str(&format!("  {} ({}, {})\n", f.name, f.type_name, req));
        out.push_str(&format!("    {}\n", f.description));
        if !f.enum_values.is_empty() {
            out.push_str(&format!("    values: {}\n", f.enum_values.join(", ")));
        }
    }
    out
}

/// Formats all entities as a Markdown list.
#[must_use]
pub fn format_entity_list_md(entities: &[EntityInfo]) -> String {
    let mut out = String::new();
    out.push_str("## Entity Types\n\n");
    for e in entities {
        out.push_str(&format!("- **{}** — {}\n", e.name, e.description));
    }
    out
}

/// Formats a single entity as a Markdown field table.
#[must_use]
pub fn format_entity_detail_md(entity: &EntityInfo) -> String {
    let mut out = String::new();
    out.push_str(&format!("## {}\n\n", entity.name));
    out.push_str(&format!("{}\n\n", entity.description));
    out.push_str("| Field | Type | Required | Values | Description |\n");
    out.push_str("|-------|------|----------|--------|-------------|\n");
    for f in &entity.fields {
        let req = if f.required { "yes" } else { "no" };
        let vals = if f.enum_values.is_empty() {
            "—".to_string()
        } else {
            f.enum_values.join(", ")
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            f.name, f.type_name, req, vals, f.description
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Asserts that the field names returned by `Describe` match the keys
    /// serde produces when serializing a sample value.
    ///
    /// This is a compile-time drift test: if a field is added to the struct
    /// but not to the `Describe` impl (or vice versa), this test fails.
    fn assert_fields_match<T: serde::Serialize + Describe>(sample: &T) {
        let value = serde_yaml::to_value(sample).expect("sample should serialize");
        let serde_keys: HashSet<&str> = value
            .as_mapping()
            .expect("sample should serialize to a mapping")
            .keys()
            .map(|k| k.as_str().expect("keys should be strings"))
            .collect();

        let describe_keys: HashSet<&str> = T::describe().fields.iter().map(|f| f.name).collect();

        assert_eq!(
            serde_keys,
            describe_keys,
            "serde keys and Describe fields must match for {}",
            std::any::type_name::<T>()
        );
    }

    #[test]
    fn drift_project() {
        let sample = crate::model::Project {
            name: "test".to_string(),
            title: "Test".to_string(),
        };
        assert_fields_match(&sample);
    }

    #[test]
    fn drift_roadmap() {
        let sample = crate::model::Roadmap {
            project: "test".to_string(),
            roadmap: "test-roadmap".to_string(),
            title: "Test Roadmap".to_string(),
            phases: vec!["phase-1-foo".to_string()],
            dependencies: Some(vec!["other".to_string()]),
        };
        assert_fields_match(&sample);
    }

    #[test]
    fn drift_phase() {
        let sample = crate::model::Phase {
            phase: 1,
            title: "Test Phase".to_string(),
            status: crate::model::PhaseStatus::Done,
            completed: Some(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
        };
        assert_fields_match(&sample);
    }

    #[test]
    fn drift_task() {
        let sample = crate::model::Task {
            project: "test".to_string(),
            title: "Test Task".to_string(),
            status: crate::model::TaskStatus::Open,
            priority: crate::model::Priority::High,
            created: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            tags: Some(vec!["tag1".to_string()]),
        };
        assert_fields_match(&sample);
    }

    #[test]
    fn all_entities_returns_four() {
        let entities = all_entities();
        assert_eq!(entities.len(), 4);
        let names: Vec<&str> = entities.iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["project", "roadmap", "phase", "task"]);
    }

    #[test]
    fn format_entity_list_contains_all_names() {
        let entities = all_entities();
        let output = format_entity_list(&entities);
        for e in &entities {
            assert!(output.contains(e.name), "missing entity: {}", e.name);
        }
    }

    #[test]
    fn format_entity_detail_shows_fields() {
        let entity = crate::model::Task::describe();
        let output = format_entity_detail(&entity);
        assert!(output.contains("task"));
        assert!(output.contains("status"));
        assert!(output.contains("priority"));
        assert!(output.contains("open, in-progress, done, wont-fix"));
    }

    #[test]
    fn format_entity_list_md_has_heading() {
        let entities = all_entities();
        let output = format_entity_list_md(&entities);
        assert!(output.contains("## Entity Types"));
        assert!(output.contains("**project**"));
    }

    #[test]
    fn format_entity_detail_md_has_table() {
        let entity = crate::model::Phase::describe();
        let output = format_entity_detail_md(&entity);
        assert!(output.contains("## phase"));
        assert!(output.contains("| Field |"));
        assert!(output.contains("not-started, in-progress, done, blocked"));
    }
}
