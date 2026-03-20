//! RFC 9457 Problem Details for HTTP APIs.
//!
//! Provides a [`ProblemDetail`] type that serializes to the standard
//! `application/problem+json` format.

use serde::Serialize;

use crate::error::Error;

/// An RFC 9457 Problem Details object.
#[derive(Debug, Clone, Serialize)]
pub struct ProblemDetail {
    /// A URI reference identifying the problem type.
    #[serde(rename = "type")]
    pub problem_type: String,
    /// A short human-readable summary of the problem.
    pub title: String,
    /// The HTTP status code.
    pub status: u16,
    /// A human-readable explanation specific to this occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// A URI reference identifying the specific occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

impl From<&Error> for ProblemDetail {
    fn from(err: &Error) -> Self {
        match err {
            Error::ProjectNotFound(name) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Not Found".to_string(),
                status: 404,
                detail: Some(format!("project not found: {name}")),
                instance: None,
            },
            Error::RoadmapNotFound(name) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Not Found".to_string(),
                status: 404,
                detail: Some(format!("roadmap not found: {name}")),
                instance: None,
            },
            Error::PhaseNotFound(name) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Not Found".to_string(),
                status: 404,
                detail: Some(format!("phase not found: {name}")),
                instance: None,
            },
            Error::TaskNotFound(name) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Not Found".to_string(),
                status: 404,
                detail: Some(format!("task not found: {name}")),
                instance: None,
            },
            Error::DuplicateSlug(slug) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Conflict".to_string(),
                status: 409,
                detail: Some(format!("'{slug}' already exists")),
                instance: None,
            },
            Error::CyclicDependency(msg) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Conflict".to_string(),
                status: 409,
                detail: Some(format!("cyclic dependency: {msg}")),
                instance: None,
            },
            Error::AlreadyInitialized => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Conflict".to_string(),
                status: 409,
                detail: Some("plan repo is already initialized".to_string()),
                instance: None,
            },
            Error::ProjectNotSpecified => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Bad Request".to_string(),
                status: 400,
                detail: Some("no project specified — use the project query parameter".to_string()),
                instance: None,
            },
            Error::InvalidPhaseSelection(msg) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Bad Request".to_string(),
                status: 400,
                detail: Some(format!("invalid phase selection: {msg}")),
                instance: None,
            },
            Error::RoadmapHasIncompletePhases(slug) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Bad Request".to_string(),
                status: 400,
                detail: Some(format!(
                    "roadmap '{slug}' has incomplete phases — pass --force to archive anyway"
                )),
                instance: None,
            },
            Error::RemoteNotFound(name) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Not Found".to_string(),
                status: 404,
                detail: Some(format!("remote not found: {name}")),
                instance: None,
            },
            Error::DuplicateRemote(name) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Conflict".to_string(),
                status: 409,
                detail: Some(format!("remote '{name}' already exists")),
                instance: None,
            },
            Error::MergeConflict(msg) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Conflict".to_string(),
                status: 409,
                detail: Some(format!("merge conflict: {msg}")),
                instance: None,
            },
            Error::NoMergeInProgress => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Conflict".to_string(),
                status: 409,
                detail: Some("no merge in progress".to_string()),
                instance: None,
            },
            Error::NotConflicted(path) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Bad Request".to_string(),
                status: 400,
                detail: Some(format!("file '{path}' is not in the unmerged list")),
                instance: None,
            },
            // Internal errors: no detail leak
            Error::Io(_)
            | Error::FrontmatterParse(_)
            | Error::FrontmatterMissing
            | Error::ConfigParse(_)
            | Error::ConfigNotFound
            | Error::ConfigSerialize(_)
            | Error::InvalidPath(_)
            | Error::PushRejected(_)
            | Error::BranchesDiverged(_)
            | Error::InvalidConfigValue { .. }
            | Error::Git(_) => ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Internal Server Error".to_string(),
                status: 500,
                detail: None,
                instance: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_detail_serializes_to_rfc9457() {
        let pd = ProblemDetail {
            problem_type: "about:blank".to_string(),
            title: "Not Found".to_string(),
            status: 404,
            detail: Some("thing not found".to_string()),
            instance: None,
        };
        let json = serde_json::to_value(&pd).unwrap();
        assert_eq!(json["type"], "about:blank");
        assert_eq!(json["title"], "Not Found");
        assert_eq!(json["status"], 404);
        assert_eq!(json["detail"], "thing not found");
    }

    #[test]
    fn problem_detail_optional_fields_omitted() {
        let pd = ProblemDetail {
            problem_type: "about:blank".to_string(),
            title: "Internal Server Error".to_string(),
            status: 500,
            detail: None,
            instance: None,
        };
        let json = serde_json::to_value(&pd).unwrap();
        let obj = json.as_object().unwrap();
        assert!(!obj.contains_key("detail"));
        assert!(!obj.contains_key("instance"));
    }

    #[test]
    fn problem_type_field_renamed() {
        let pd = ProblemDetail {
            problem_type: "urn:example:custom".to_string(),
            title: "Custom".to_string(),
            status: 422,
            detail: None,
            instance: None,
        };
        let json = serde_json::to_value(&pd).unwrap();
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("type"));
        assert!(!obj.contains_key("problem_type"));
    }

    #[test]
    fn from_project_not_found() {
        let err = Error::ProjectNotFound("foo".to_string());
        let pd = ProblemDetail::from(&err);
        assert_eq!(pd.status, 404);
        assert_eq!(pd.title, "Not Found");
        assert!(pd.detail.as_ref().unwrap().contains("foo"));
    }

    #[test]
    fn from_duplicate_slug() {
        let err = Error::DuplicateSlug("bar".to_string());
        let pd = ProblemDetail::from(&err);
        assert_eq!(pd.status, 409);
        assert_eq!(pd.title, "Conflict");
    }

    #[test]
    fn from_project_not_specified() {
        let err = Error::ProjectNotSpecified;
        let pd = ProblemDetail::from(&err);
        assert_eq!(pd.status, 400);
        assert_eq!(pd.title, "Bad Request");
    }

    #[test]
    fn from_io_error() {
        let err = Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "oops"));
        let pd = ProblemDetail::from(&err);
        assert_eq!(pd.status, 500);
        assert!(pd.detail.is_none(), "should not leak internal details");
    }
}
