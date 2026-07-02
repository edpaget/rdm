//! Review-document operations: list stored reviews.
//!
//! Not to be confused with [`crate::ops::review`], which enumerates plan
//! items awaiting review (the needs-review queue). This module operates on
//! the [`Review`] documents stored under a project's `reviews/` directory.

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::Review;
use crate::store::{DirEntryKind, Store};

/// Lists all reviews for a project, sorted by review id.
///
/// Returns `(id, Document<Review>)` tuples. Returns an empty vec if the
/// reviews directory doesn't exist.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project does not exist,
/// [`Error::Io`] if the directory cannot be read, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a
/// review file has invalid frontmatter.
pub fn list_reviews(store: &impl Store, project: &str) -> Result<Vec<(String, Document<Review>)>> {
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    let dir = crate::paths::reviews_dir(project);
    let entries = store.list(&dir)?;

    let mut reviews: Vec<(String, Document<Review>)> = Vec::new();
    for entry in entries {
        if entry.kind != DirEntryKind::File {
            continue;
        }
        if !entry.name.ends_with(".md") {
            continue;
        }
        let id = entry.name.trim_end_matches(".md").to_string();
        let doc = crate::io::load_review(store, project, &id)?;
        reviews.push((id, doc));
    }
    reviews.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(reviews)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Project, ReviewState, ReviewTarget};
    use crate::store::MemoryStore;
    use chrono::TimeZone;

    fn setup_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        let project_doc = Document {
            frontmatter: Project {
                name: "test".to_string(),
                title: "Test Project".to_string(),
            },
            body: String::new(),
        };
        store
            .write(
                &crate::paths::project_md_path("test"),
                project_doc.render().unwrap(),
            )
            .unwrap();
        store
    }

    fn sample_review(id: &str) -> Document<Review> {
        Document {
            frontmatter: Review {
                id: id.to_string(),
                author: "ed".to_string(),
                target: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                state: ReviewState::Draft,
                verdict: None,
                created: chrono::Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 0).unwrap(),
                submitted: None,
                created_commit: None,
                comments: vec![],
            },
            body: "Review summary.".to_string(),
        }
    }

    #[test]
    fn list_reviews_sorted_by_id() {
        let mut store = setup_store();
        for id in [
            "2026-07-01-1430-zz99",
            "2026-06-30-0900-aa11",
            "2026-07-01-0800-mm55",
        ] {
            let doc = sample_review(id);
            crate::io::write_review(&mut store, "test", id, &doc).unwrap();
        }
        let reviews = list_reviews(&store, "test").unwrap();
        assert_eq!(reviews.len(), 3);
        assert_eq!(reviews[0].0, "2026-06-30-0900-aa11");
        assert_eq!(reviews[1].0, "2026-07-01-0800-mm55");
        assert_eq!(reviews[2].0, "2026-07-01-1430-zz99");
        assert_eq!(reviews[0].1.frontmatter.id, "2026-06-30-0900-aa11");
    }

    #[test]
    fn list_reviews_missing_dir_is_empty() {
        let store = setup_store();
        let reviews = list_reviews(&store, "test").unwrap();
        assert!(reviews.is_empty());
    }

    #[test]
    fn list_reviews_project_not_found() {
        let store = MemoryStore::new();
        let result = list_reviews(&store, "nonexistent");
        assert!(matches!(result, Err(Error::ProjectNotFound(_))));
    }
}
