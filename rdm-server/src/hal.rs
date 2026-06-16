//! HAL (Hypertext Application Language) response types for rdm APIs.
//!
//! Implements a subset of the [HAL specification](https://datatracker.ietf.org/doc/html/draft-kelly-json-hal)
//! sufficient for rdm's REST API responses.

use std::collections::HashMap;

use serde::Serialize;

/// A HAL link object.
#[derive(Debug, Clone, Serialize)]
pub struct HalLink {
    /// The target URI of the link.
    pub href: String,
    /// An optional human-readable title for the link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Whether the href is a URI template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub templated: Option<bool>,
}

impl HalLink {
    /// Creates a new link with the given href.
    pub fn new(href: impl Into<String>) -> Self {
        HalLink {
            href: href.into(),
            title: None,
            templated: None,
        }
    }
}

/// A HAL resource wrapping domain data of type `T`.
///
/// The domain data fields are flattened into the top-level JSON object
/// alongside `_links` and `_embedded`.
#[derive(Debug, Clone, Serialize)]
pub struct HalResource<T: Serialize> {
    /// HAL links keyed by relation name. Always includes at least `"self"`.
    pub _links: HashMap<String, HalLink>,

    /// Embedded sub-resources keyed by relation name.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub _embedded: HashMap<String, Vec<serde_json::Value>>,

    /// The domain data, flattened into the resource object.
    #[serde(flatten)]
    pub data: T,
}

impl<T: Serialize> HalResource<T> {
    /// Creates a new HAL resource with a `self` link.
    pub fn new(data: T, self_href: impl Into<String>) -> Self {
        let mut links = HashMap::new();
        links.insert("self".to_string(), HalLink::new(self_href));
        HalResource {
            _links: links,
            _embedded: HashMap::new(),
            data,
        }
    }

    /// Adds a link with the given relation name.
    pub fn with_link(mut self, rel: impl Into<String>, link: HalLink) -> Self {
        self._links.insert(rel.into(), link);
        self
    }

    /// Adds embedded resources under the given relation name.
    pub fn with_embedded(mut self, rel: impl Into<String>, items: Vec<serde_json::Value>) -> Self {
        self._embedded.insert(rel.into(), items);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hal_link_serializes_minimal() {
        let link = HalLink::new("/things/1");
        let json = serde_json::to_value(&link).unwrap();
        assert_eq!(json, serde_json::json!({"href": "/things/1"}));
        assert!(!json.as_object().unwrap().contains_key("title"));
        assert!(!json.as_object().unwrap().contains_key("templated"));
    }

    #[test]
    fn hal_link_serializes_all_fields() {
        let link = HalLink {
            href: "/items/{id}".to_string(),
            title: Some("An item".to_string()),
            templated: Some(true),
        };
        let json = serde_json::to_value(&link).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "href": "/items/{id}",
                "title": "An item",
                "templated": true
            })
        );
    }

    #[derive(Serialize)]
    struct Widget {
        name: String,
        count: u32,
    }

    #[test]
    fn hal_resource_serializes_with_self_link() {
        let resource = HalResource::new(
            Widget {
                name: "sprocket".into(),
                count: 5,
            },
            "/widgets/sprocket",
        );
        let json = serde_json::to_value(&resource).unwrap();
        assert_eq!(json["name"], "sprocket");
        assert_eq!(json["count"], 5);
        assert_eq!(json["_links"]["self"]["href"], "/widgets/sprocket");
    }

    #[test]
    fn hal_resource_omits_empty_embedded() {
        let resource = HalResource::new(
            Widget {
                name: "gear".into(),
                count: 1,
            },
            "/widgets/gear",
        );
        let json = serde_json::to_value(&resource).unwrap();
        assert!(!json.as_object().unwrap().contains_key("_embedded"));
    }

    #[test]
    fn hal_resource_with_embedded() {
        let resource = HalResource::new(
            Widget {
                name: "box".into(),
                count: 3,
            },
            "/widgets/box",
        )
        .with_embedded(
            "parts",
            vec![
                serde_json::json!({"name": "screw"}),
                serde_json::json!({"name": "nail"}),
            ],
        );
        let json = serde_json::to_value(&resource).unwrap();
        let parts = json["_embedded"]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["name"], "screw");
    }

    #[test]
    fn hal_resource_builder_chain() {
        let resource = HalResource::new(
            Widget {
                name: "thing".into(),
                count: 0,
            },
            "/widgets/thing",
        )
        .with_link(
            "collection",
            HalLink {
                href: "/widgets".to_string(),
                title: Some("All widgets".to_string()),
                templated: None,
            },
        )
        .with_embedded("related", vec![serde_json::json!({"id": 1})]);

        let json = serde_json::to_value(&resource).unwrap();
        assert_eq!(json["_links"]["self"]["href"], "/widgets/thing");
        assert_eq!(json["_links"]["collection"]["href"], "/widgets");
        assert_eq!(json["_links"]["collection"]["title"], "All widgets");
        assert_eq!(json["_embedded"]["related"][0]["id"], 1);
    }
}
