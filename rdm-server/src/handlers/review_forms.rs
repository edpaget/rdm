//! Plain-HTML-form endpoints for authoring reviews from the detail pages.
//!
//! These routes exist because native `<form>` elements only speak
//! `GET`/`POST` with urlencoded bodies, while the review REST API
//! (`crate::handlers::reviews`) is JSON-only and needs `PATCH`/`DELETE`
//! for half its operations. Each handler here is a thin urlencoded shim
//! over the same `rdm_core::ops::reviews` operations — no business rule is
//! re-implemented at this layer.
//!
//! Every action follows Post/Redirect/Get: on success it `303`s back to
//! the reviewed document's detail page (with a `#review-draft` or
//! `#review-<id>` fragment); on a validation or lifecycle error it `303`s
//! back with the actionable error message in a `?draft_error=` query
//! parameter, which the detail page renders as an inline banner — never a
//! raw Problem+JSON body. Only requests that cannot name a page to return
//! to (unknown review id, malformed target reference from a tampered
//! hidden field) render the HTML error page directly.
//!
//! # Author identity
//!
//! There is no authentication. The visitor's name comes from the start
//! form's `author` field and is remembered in an `rdm_author` cookie
//! (percent-encoded, first `rdm_author=` pair wins when the `Cookie`
//! header carries several, and a blank or undecodable value counts as no
//! identity at all). The cookie only scopes which *draft* the panel
//! resumes — it is a convenience, not a security boundary.
//!
//! # Accepted limitations
//!
//! Consistent with the JSON API's no-auth posture, there are no ownership
//! checks anywhere in the server: any visitor can dismiss any submitted
//! review or delete any draft. Additionally, unlike the JSON API (which
//! rejects a blank submit `summary` with a 422), the submit form treats a
//! blank summary textarea as "keep the current summary" — the textarea is
//! pre-filled, so deliberate clearing is not possible from this form (use
//! the CLI or JSON API).

use axum::extract::{Form, Path, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::Response;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde::Deserialize;

use rdm_core::model::{CommentDoc, CommentDocKind, ReviewState, Verdict};
use rdm_core::ops::BodyUpdate;
use rdm_core::ops::reviews::{
    AddComment, AnchorUpdate, CreateReview, DocUpdate, ReviewFilter, ReviewTransition,
    UpdateComment,
};
use rdm_store_fs::FsStore;

use crate::content_type::ResponseFormat;
use crate::error::error_response;
use crate::extract::see_other_response;
use crate::review_views::target_detail_href;
use crate::state::AppState;

/// Name of the cookie remembering the visitor's review-author identity.
const AUTHOR_COOKIE: &str = "rdm_author";

/// Fragment for draft-panel actions: the panel's DOM id on detail pages.
const DRAFT_FRAGMENT: &str = "review-draft";

/// Percent-encoding set for cookie and query values: everything outside
/// RFC 3986's unreserved characters (alphanumerics, `-`, `.`, `_`, `~`).
const ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Percent-encodes a value for a cookie or query-string slot.
fn encode_value(value: &str) -> String {
    utf8_percent_encode(value, ENCODE_SET).to_string()
}

/// Reads the visitor's author identity from the `rdm_author` cookie.
///
/// Precedence: the first `rdm_author=` pair across the request's `Cookie`
/// headers wins; later duplicates are ignored. A value that fails
/// percent-decoding, is not UTF-8, or is blank after trimming counts as
/// **absent** identity (`None`) — it never becomes an empty author.
pub fn read_author_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|header| header.split(';'))
        .find_map(|pair| pair.trim().strip_prefix("rdm_author="))?;
    let decoded = percent_decode_str(raw).decode_utf8().ok()?;
    let trimmed = decoded.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Builds the `Set-Cookie` header remembering the author identity for a
/// year. `HttpOnly` (no script access needed), `SameSite=Lax`, path-wide.
fn author_cookie_header(author: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{AUTHOR_COOKIE}={}; Path=/; Max-Age=31536000; SameSite=Lax; HttpOnly",
        encode_value(author)
    ))
    .expect("percent-encoded cookie value is always a valid header value")
}

/// `303 See Other` back to the detail page with a fragment.
fn redirect_ok(target_href: &str, fragment: &str) -> Response {
    see_other_response(&format!("{target_href}#{fragment}"))
}

/// `303 See Other` back to the detail page carrying an inline error
/// message in `?draft_error=`.
fn redirect_err(target_href: &str, fragment: &str, message: &str) -> Response {
    see_other_response(&format!(
        "{target_href}?draft_error={}#{fragment}",
        encode_value(message)
    ))
}

/// Resolves the detail-page href of the document a review targets — the
/// redirect destination for every action on an existing review.
///
/// When the review itself cannot be loaded (unknown id, malformed file)
/// there is no page to return to, so this yields the HTML error page as
/// the `Err` response.
#[allow(clippy::result_large_err)]
fn review_target_href(store: &FsStore, project: &str, review_id: &str) -> Result<String, Response> {
    let doc = rdm_core::ops::reviews::get_review(store, project, review_id)
        .map_err(|e| error_response(e, ResponseFormat::Html))?;
    Ok(target_detail_href(project, &doc.frontmatter.target))
}

/// Maps the flat `doc_stem` form field to a comment scope: blank or absent
/// means whole-document, anything else names a phase stem.
fn comment_doc_from_stem(doc_stem: &str) -> Option<CommentDoc> {
    let stem = doc_stem.trim();
    (!stem.is_empty()).then(|| CommentDoc {
        kind: CommentDocKind::Phase,
        stem: stem.to_string(),
    })
}

/// Form body for `POST /projects/:project/reviews/form`.
#[derive(Deserialize)]
pub struct StartReviewForm {
    /// The page document's target reference (hidden field).
    #[serde(default)]
    target: String,
    /// The visitor's name.
    #[serde(default)]
    author: String,
}

/// `POST /projects/:project/reviews/form` — start (or resume) the
/// visitor's draft review on a document.
///
/// If an open draft by the same author already exists on the target it is
/// resumed instead of duplicated. Sets the `rdm_author` cookie on success.
pub async fn start_review_form(
    State(state): State<AppState>,
    Path(project): Path<String>,
    Form(req): Form<StartReviewForm>,
) -> Response {
    let mut store = state.store();
    // A bad target reference means a tampered or stale hidden field — there
    // is no trustworthy page to bounce back to, so error page it is.
    let target =
        match rdm_core::ops::reviews::parse_review_target_ref(&store, &project, &req.target) {
            Ok(t) => t,
            Err(e) => return error_response(e, ResponseFormat::Html),
        };
    let target_href = target_detail_href(&project, &target);

    let author = req.author.trim().to_string();
    if author.is_empty() {
        return redirect_err(
            &target_href,
            DRAFT_FRAGMENT,
            "your name is required to start a review — enter it and try again",
        );
    }

    // Resume semantics: one open draft per (document, author).
    let existing = match rdm_core::ops::reviews::list_reviews(&store, &project) {
        Ok(all) => rdm_core::ops::reviews::filter_reviews(
            all,
            &ReviewFilter {
                target: Some(target.clone()),
                state: Some(ReviewState::Draft),
                verdict: None,
                author: Some(author.clone()),
            },
        )
        .into_iter()
        .next(),
        Err(e) => return redirect_err(&target_href, DRAFT_FRAGMENT, &e.to_string()),
    };
    if existing.is_none() {
        let created = rdm_core::ops::mutate(&mut store, &project, |s| {
            rdm_core::ops::reviews::create_review(
                s,
                CreateReview {
                    project: &project,
                    author: &author,
                    target: target.clone(),
                    body: None,
                },
            )
        });
        if let Err(e) = created {
            return redirect_err(&target_href, DRAFT_FRAGMENT, &e.to_string());
        }
    }

    let mut response = redirect_ok(&target_href, DRAFT_FRAGMENT);
    response
        .headers_mut()
        .insert(SET_COOKIE, author_cookie_header(&author));
    response
}

/// Form body for `POST /projects/:project/reviews/:review_id/form/comments`.
#[derive(Deserialize)]
pub struct AddCommentForm {
    /// Comment text (Markdown). Must not be blank.
    #[serde(default)]
    body: String,
    /// Optional phase stem scoping the comment (roadmap reviews only);
    /// blank means whole-document.
    #[serde(default)]
    doc_stem: String,
}

/// `POST /projects/:project/reviews/:review_id/form/comments` — add a
/// whole-document (or `doc`-scoped) comment to the draft.
pub async fn add_comment_form(
    State(state): State<AppState>,
    Path((project, review_id)): Path<(String, String)>,
    Form(req): Form<AddCommentForm>,
) -> Response {
    let mut store = state.store();
    let target_href = match review_target_href(&store, &project, &review_id) {
        Ok(href) => href,
        Err(response) => return response,
    };
    if req.body.trim().is_empty() {
        return redirect_err(
            &target_href,
            DRAFT_FRAGMENT,
            "comment body must not be empty",
        );
    }
    let result = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::reviews::add_comment(
            s,
            AddComment {
                project: &project,
                review_id: &review_id,
                body: &req.body,
                doc: comment_doc_from_stem(&req.doc_stem),
                anchor: None,
            },
        )
    });
    match result {
        Ok(_) => redirect_ok(&target_href, DRAFT_FRAGMENT),
        Err(e) => redirect_err(&target_href, DRAFT_FRAGMENT, &e.to_string()),
    }
}

/// Form body for
/// `POST /projects/:project/reviews/:review_id/form/comments/:comment_id/edit`.
///
/// Full-state replacement (matching the other web edit forms): `body` is
/// the complete new text and `doc_stem` the complete new scope (blank =
/// whole document). Anchors are untouched by this form.
///
/// A posted scope equal to the comment's current one maps to
/// [`DocUpdate::Keep`] rather than a re-`Set`: core revalidates `Set`
/// against the live phase list, so an untouched (possibly stale,
/// deleted-phase) scope must not be re-asserted just because the select
/// posted its value back.
#[derive(Deserialize)]
pub struct EditCommentForm {
    /// Replacement comment text (Markdown). Must not be blank.
    #[serde(default)]
    body: String,
    /// Replacement phase scope; blank clears back to whole-document.
    #[serde(default)]
    doc_stem: String,
}

/// `POST /projects/:project/reviews/:review_id/form/comments/:comment_id/edit`
/// — replace a pending comment's text and scope.
pub async fn edit_comment_form(
    State(state): State<AppState>,
    Path((project, review_id, comment_id)): Path<(String, String, String)>,
    Form(req): Form<EditCommentForm>,
) -> Response {
    let mut store = state.store();
    // Loaded (not just href-resolved) so the posted scope can be compared
    // against the comment's current one below.
    let review = match rdm_core::ops::reviews::get_review(&store, &project, &review_id) {
        Ok(doc) => doc,
        Err(e) => return error_response(e, ResponseFormat::Html),
    };
    let target_href = target_detail_href(&project, &review.frontmatter.target);
    let Ok(comment_id) = comment_id.parse::<u32>() else {
        // Comment ids are template-generated; a non-numeric one is a
        // tampered URL, not a user mistake worth a friendly banner.
        return error_response(
            rdm_core::error::Error::CommentNotFound {
                review_id: review_id.clone(),
                comment_id: 0,
            },
            ResponseFormat::Html,
        );
    };
    if req.body.trim().is_empty() {
        return redirect_err(
            &target_href,
            DRAFT_FRAGMENT,
            "comment body must not be empty",
        );
    }
    let posted_doc = comment_doc_from_stem(&req.doc_stem);
    let current_doc = review
        .frontmatter
        .comments
        .iter()
        .find(|c| c.id == comment_id)
        .and_then(|c| c.doc.clone());
    let doc = if posted_doc == current_doc {
        DocUpdate::Keep
    } else {
        match posted_doc {
            Some(d) => DocUpdate::Set(d),
            None => DocUpdate::Clear,
        }
    };
    let result = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::reviews::update_comment(
            s,
            UpdateComment {
                project: &project,
                review_id: &review_id,
                comment_id,
                body: Some(&req.body),
                anchor: AnchorUpdate::Keep,
                doc,
                status: None,
                applied_commit: None,
                reply: None,
            },
        )
    });
    match result {
        Ok(_) => redirect_ok(&target_href, DRAFT_FRAGMENT),
        Err(e) => redirect_err(&target_href, DRAFT_FRAGMENT, &e.to_string()),
    }
}

/// `POST /projects/:project/reviews/:review_id/form/comments/:comment_id/remove`
/// — remove a pending comment from the draft.
pub async fn remove_comment_form(
    State(state): State<AppState>,
    Path((project, review_id, comment_id)): Path<(String, String, String)>,
) -> Response {
    let mut store = state.store();
    let target_href = match review_target_href(&store, &project, &review_id) {
        Ok(href) => href,
        Err(response) => return response,
    };
    let Ok(comment_id) = comment_id.parse::<u32>() else {
        return error_response(
            rdm_core::error::Error::CommentNotFound {
                review_id: review_id.clone(),
                comment_id: 0,
            },
            ResponseFormat::Html,
        );
    };
    let result = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::reviews::remove_comment(s, &project, &review_id, comment_id)
    });
    match result {
        Ok(_) => redirect_ok(&target_href, DRAFT_FRAGMENT),
        Err(e) => redirect_err(&target_href, DRAFT_FRAGMENT, &e.to_string()),
    }
}

/// Form body for `POST /projects/:project/reviews/:review_id/form/submit`.
#[derive(Deserialize)]
pub struct SubmitReviewForm {
    /// Verdict radio value; empty when none was picked (core rejects it).
    #[serde(default)]
    verdict: String,
    /// Summary textarea. Blank keeps the current summary (see the module
    /// docs for why this diverges from the JSON API's 422).
    #[serde(default)]
    summary: String,
}

/// `POST /projects/:project/reviews/:review_id/form/submit` — submit the
/// draft with a verdict, making it public in the Reviews section.
pub async fn submit_review_form(
    State(state): State<AppState>,
    Path((project, review_id)): Path<(String, String)>,
    Form(req): Form<SubmitReviewForm>,
) -> Response {
    let mut store = state.store();
    let target_href = match review_target_href(&store, &project, &review_id) {
        Ok(href) => href,
        Err(response) => return response,
    };
    let verdict: Option<Verdict> = match req.verdict.trim() {
        "" => None,
        v => match v.parse() {
            Ok(v) => Some(v),
            Err(_) => {
                return redirect_err(
                    &target_href,
                    DRAFT_FRAGMENT,
                    &format!(
                        "invalid verdict: '{v}' (expected approve, request-changes, or comment)"
                    ),
                );
            }
        },
    };
    let result = rdm_core::ops::mutate(&mut store, &project, |s| {
        if !req.summary.trim().is_empty() {
            rdm_core::ops::reviews::set_summary(
                s,
                &project,
                &review_id,
                BodyUpdate::Set(req.summary.clone()),
            )?;
        }
        rdm_core::ops::reviews::submit_review(s, &project, &review_id, verdict)
    });
    match result {
        Ok(_) => redirect_ok(&target_href, &format!("review-{review_id}")),
        Err(e) => redirect_err(&target_href, DRAFT_FRAGMENT, &e.to_string()),
    }
}

/// `POST /projects/:project/reviews/:review_id/form/dismiss` — dismiss a
/// submitted review from its public card.
pub async fn dismiss_review_form(
    State(state): State<AppState>,
    Path((project, review_id)): Path<(String, String)>,
) -> Response {
    let mut store = state.store();
    let target_href = match review_target_href(&store, &project, &review_id) {
        Ok(href) => href,
        Err(response) => return response,
    };
    let fragment = format!("review-{review_id}");
    let result = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::reviews::update_review(s, &project, &review_id, ReviewTransition::Dismissed)
    });
    match result {
        Ok(_) => redirect_ok(&target_href, &fragment),
        Err(e) => redirect_err(&target_href, &fragment, &e.to_string()),
    }
}

/// `POST /projects/:project/reviews/:review_id/form/delete` — delete the
/// visitor's draft (submitted reviews are part of the record; core
/// refuses them and the error surfaces inline).
pub async fn delete_review_form(
    State(state): State<AppState>,
    Path((project, review_id)): Path<(String, String)>,
) -> Response {
    let mut store = state.store();
    // Resolve the redirect destination *before* deleting the review.
    let target_href = match review_target_href(&store, &project, &review_id) {
        Ok(href) => href,
        Err(response) => return response,
    };
    let result = rdm_core::ops::mutate(&mut store, &project, |s| {
        rdm_core::ops::reviews::delete_review(s, &project, &review_id, false)
    });
    match result {
        Ok(()) => redirect_ok(&target_href, DRAFT_FRAGMENT),
        Err(e) => redirect_err(&target_href, DRAFT_FRAGMENT, &e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Request;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use rdm_core::model::ReviewState;

    use super::read_author_cookie;
    use crate::router::build_router;
    use crate::state::AppState;

    /// A plan repo with project `demo`, roadmap `alpha` (one phase,
    /// `phase-1-one`), and task `fix-login`.
    fn setup() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let mut store = rdm_store_fs::FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "demo",
                slug: "alpha",
                title: "Alpha",
                body: Some("Roadmap body.\n"),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "demo",
                roadmap: "alpha",
                slug: "one",
                title: "One",
                number: Some(1),
                body: Some("Phase body.\n"),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project: "demo",
                slug: "fix-login",
                title: "Fix login",
                body: Some("Task body.\n"),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::store::Store::commit(&mut store).unwrap();
        let state = AppState {
            plan_root: dir.path().to_path_buf(),
            quick_filters: Vec::new(),
        };
        (dir, state)
    }

    fn post_form(uri: &str, body: &str) -> Request<axum::body::Body> {
        Request::post(uri)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    async fn send(state: &AppState, req: Request<axum::body::Body>) -> axum::response::Response {
        build_router(state.clone()).oneshot(req).await.unwrap()
    }

    /// The `Location` header, percent-decoded for message assertions.
    fn decoded_location(response: &axum::response::Response) -> String {
        let raw = response
            .headers()
            .get("location")
            .expect("redirect must carry Location")
            .to_str()
            .unwrap();
        percent_encoding::percent_decode_str(raw)
            .decode_utf8()
            .unwrap()
            .to_string()
    }

    /// All reviews in the demo project, straight from core.
    fn reviews(
        state: &AppState,
    ) -> Vec<(
        String,
        rdm_core::document::Document<rdm_core::model::Review>,
    )> {
        rdm_core::ops::reviews::list_reviews(&state.store(), "demo").unwrap()
    }

    /// Starts a draft on `target` by `author` via the form route; returns
    /// its id.
    async fn start_draft(state: &AppState, target: &str, author: &str) -> String {
        let body = format!("target={}&author={author}", target.replace('/', "%2F"));
        let response = send(state, post_form("/projects/demo/reviews/form", &body)).await;
        assert_eq!(response.status(), 303);
        reviews(state)
            .into_iter()
            .find(|(_, doc)| {
                doc.frontmatter.author == author
                    && doc.frontmatter.state == ReviewState::Draft
                    && doc.frontmatter.target.label() == target
            })
            .map(|(id, _)| id)
            .expect("draft created")
    }

    // -- cookies --

    #[test]
    fn read_author_cookie_first_pair_wins() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "rdm_author=first; other=x; rdm_author=second"
                .parse()
                .unwrap(),
        );
        assert_eq!(read_author_cookie(&headers).as_deref(), Some("first"));
    }

    #[test]
    fn read_author_cookie_first_pair_wins_across_headers() {
        let mut headers = axum::http::HeaderMap::new();
        headers.append(
            axum::http::header::COOKIE,
            "rdm_author=one".parse().unwrap(),
        );
        headers.append(
            axum::http::header::COOKIE,
            "rdm_author=two".parse().unwrap(),
        );
        assert_eq!(read_author_cookie(&headers).as_deref(), Some("one"));
    }

    #[test]
    fn read_author_cookie_invalid_utf8_is_absent() {
        // %ff%fe percent-decodes to bytes that are not valid UTF-8: no
        // identity, not a mangled one.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "rdm_author=%ff%fe".parse().unwrap(),
        );
        assert_eq!(read_author_cookie(&headers), None);
    }

    #[test]
    fn read_author_cookie_blank_value_is_absent() {
        for raw in ["rdm_author=", "rdm_author=%20%20", "rdm_author=+"] {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(axum::http::header::COOKIE, raw.parse().unwrap());
            // `+` is not decoded as a space in cookies, so the last case is
            // a literal plus — present. Only the first two are blank.
            if raw == "rdm_author=+" {
                assert_eq!(read_author_cookie(&headers).as_deref(), Some("+"));
            } else {
                assert_eq!(read_author_cookie(&headers), None, "raw: {raw}");
            }
        }
    }

    #[test]
    fn read_author_cookie_decodes_percent_encoding() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "rdm_author=Ed%20Paget%20%F0%9F%8E%89".parse().unwrap(),
        );
        assert_eq!(
            read_author_cookie(&headers).as_deref(),
            Some("Ed Paget \u{1F389}")
        );
    }

    #[test]
    fn read_author_cookie_missing_is_none() {
        let headers = axum::http::HeaderMap::new();
        assert_eq!(read_author_cookie(&headers), None);
    }

    #[tokio::test]
    async fn author_cookie_round_trips_through_set_cookie() {
        let (_dir, state) = setup();
        let response = send(
            &state,
            post_form(
                "/projects/demo/reviews/form",
                "target=task%2Ffix-login&author=Ed%20Paget",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let set_cookie = response
            .headers()
            .get("set-cookie")
            .expect("start must set the author cookie")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            set_cookie.starts_with("rdm_author=Ed%20Paget;"),
            "{set_cookie}"
        );
        assert!(set_cookie.contains("SameSite=Lax"), "{set_cookie}");
        // The value read back is the original name.
        let mut headers = axum::http::HeaderMap::new();
        let pair = set_cookie.split(';').next().unwrap();
        headers.insert(axum::http::header::COOKIE, pair.parse().unwrap());
        assert_eq!(read_author_cookie(&headers).as_deref(), Some("Ed Paget"));
    }

    // -- start --

    #[tokio::test]
    async fn start_review_form_creates_draft_and_redirects() {
        let (_dir, state) = setup();
        let response = send(
            &state,
            post_form(
                "/projects/demo/reviews/form",
                "target=task%2Ffix-login&author=ed",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        assert_eq!(
            decoded_location(&response),
            "/projects/demo/tasks/fix-login#review-draft"
        );
        let all = reviews(&state);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].1.frontmatter.author, "ed");
        assert_eq!(all[0].1.frontmatter.state, ReviewState::Draft);
    }

    #[tokio::test]
    async fn start_review_form_resumes_existing_draft_same_author() {
        let (_dir, state) = setup();
        start_draft(&state, "task/fix-login", "ed").await;
        start_draft(&state, "task/fix-login", "ed").await;
        assert_eq!(reviews(&state).len(), 1, "same author+target must resume");
    }

    #[tokio::test]
    async fn start_review_form_different_author_creates_separate_draft() {
        let (_dir, state) = setup();
        start_draft(&state, "task/fix-login", "ed").await;
        start_draft(&state, "task/fix-login", "bot").await;
        assert_eq!(reviews(&state).len(), 2);
    }

    #[tokio::test]
    async fn start_review_form_blank_author_redirects_with_error() {
        let (_dir, state) = setup();
        let response = send(
            &state,
            post_form(
                "/projects/demo/reviews/form",
                "target=task%2Ffix-login&author=%20%20",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let location = decoded_location(&response);
        assert!(location.contains("draft_error="), "{location}");
        assert!(location.contains("your name is required"), "{location}");
        assert!(reviews(&state).is_empty(), "no phantom empty-author draft");
        assert!(
            response.headers().get("set-cookie").is_none(),
            "blank identity must not be remembered"
        );
    }

    #[tokio::test]
    async fn start_review_form_invalid_target_returns_html_error() {
        let (_dir, state) = setup();
        let response = send(
            &state,
            post_form("/projects/demo/reviews/form", "target=widget%2Fx&author=ed"),
        )
        .await;
        assert_eq!(response.status(), 400);
        assert!(
            response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("text/html")
        );
    }

    // -- add comment --

    #[tokio::test]
    async fn add_comment_form_whole_document_succeeds() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=General%20note.&doc_stem=",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        assert_eq!(
            decoded_location(&response),
            "/projects/demo/tasks/fix-login#review-draft"
        );
        let doc = rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).unwrap();
        assert_eq!(doc.frontmatter.comments.len(), 1);
        assert_eq!(doc.frontmatter.comments[0].body, "General note.");
        assert!(doc.frontmatter.comments[0].doc.is_none());
    }

    #[tokio::test]
    async fn add_comment_form_doc_scoped_on_roadmap_review_succeeds() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "roadmap/alpha", "ed").await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=Phase-scoped.&doc_stem=phase-1-one",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let doc = rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).unwrap();
        let comment = &doc.frontmatter.comments[0];
        assert_eq!(comment.doc.as_ref().unwrap().stem, "phase-1-one");
    }

    #[tokio::test]
    async fn add_comment_form_blank_body_redirects_with_error() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=%20%20",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let location = decoded_location(&response);
        assert!(location.contains("must not be empty"), "{location}");
        let doc = rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).unwrap();
        assert!(doc.frontmatter.comments.is_empty());
    }

    #[tokio::test]
    async fn add_comment_form_doc_scope_on_task_review_redirects_with_error() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=Bad%20scope.&doc_stem=phase-1-one",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let location = decoded_location(&response);
        assert!(location.contains("roadmap review"), "{location}");
    }

    #[tokio::test]
    async fn add_comment_form_unknown_review_returns_html_404() {
        let (_dir, state) = setup();
        let response = send(
            &state,
            post_form("/projects/demo/reviews/nope/form/comments", "body=x"),
        )
        .await;
        assert_eq!(response.status(), 404);
        assert!(
            response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("text/html")
        );
    }

    // -- edit comment --

    #[tokio::test]
    async fn edit_comment_form_updates_body_and_scope() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "roadmap/alpha", "ed").await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=Original.&doc_stem=phase-1-one",
            ),
        )
        .await;
        // Reword and clear the scope back to whole-document.
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments/1/edit"),
                "body=Reworded.&doc_stem=",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let doc = rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).unwrap();
        assert_eq!(doc.frontmatter.comments[0].body, "Reworded.");
        assert!(doc.frontmatter.comments[0].doc.is_none());

        // And set a scope again.
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments/1/edit"),
                "body=Reworded.&doc_stem=phase-1-one",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let doc = rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).unwrap();
        assert_eq!(
            doc.frontmatter.comments[0].doc.as_ref().unwrap().stem,
            "phase-1-one"
        );
    }

    #[tokio::test]
    async fn edit_comment_form_after_submit_redirects_with_error() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=Note.",
            ),
        )
        .await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/submit"),
                "verdict=comment&summary=",
            ),
        )
        .await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments/1/edit"),
                "body=Too%20late.",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let location = decoded_location(&response);
        assert!(location.contains("not a draft"), "{location}");
    }

    #[tokio::test]
    async fn edit_comment_form_non_numeric_id_returns_html_error() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments/first/edit"),
                "body=x",
            ),
        )
        .await;
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn edit_comment_form_nonexistent_comment_id_redirects_with_error() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments/9/edit"),
                "body=Valid%20text.",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let location = decoded_location(&response);
        assert!(location.contains("comment 9 not found"), "{location}");
    }

    /// Fetches a detail page as the given cookie identity.
    async fn get_html_with_cookie(state: &AppState, uri: &str, cookie: &str) -> String {
        let response = send(
            state,
            Request::get(uri)
                .header("accept", "text/html")
                .header("cookie", cookie)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), 200);
        let body = axum::body::to_bytes(response.into_body(), 262144)
            .await
            .unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }

    /// A draft comment scoped to a phase that is later deleted must keep
    /// its scope: the edit form renders a synthetic selected option for
    /// the stale stem, and a body-only edit that posts it back leaves
    /// `doc` untouched on disk instead of silently clearing it.
    #[tokio::test]
    async fn edit_comment_form_preserves_stale_phase_scope_after_phase_deleted() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "roadmap/alpha", "ed").await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=Scoped%20note.&doc_stem=phase-1-one",
            ),
        )
        .await;

        // Delete the phase out from under the comment.
        {
            let mut store = state.store();
            rdm_core::store::Store::delete(
                &mut store,
                &rdm_core::paths::phase_path("demo", "alpha", "phase-1-one"),
            )
            .unwrap();
            rdm_core::store::Store::commit(&mut store).unwrap();
        }

        // The edit form renders a synthetic selected option for the stale
        // stem (a select always posts a value; without this the browser
        // would default to "Whole roadmap" and clear the scope).
        let html =
            get_html_with_cookie(&state, "/projects/demo/roadmaps/alpha", "rdm_author=ed").await;
        assert!(
            html.contains(
                r#"<option value="phase-1-one" selected>Phase phase-1-one (deleted)</option>"#
            ),
            "synthetic stale-stem option must render selected: {html}"
        );

        // A body-only edit (the browser reposts the selected stale stem)
        // succeeds and the scope survives on disk.
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments/1/edit"),
                "body=Reworded%20note.&doc_stem=phase-1-one",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let location = decoded_location(&response);
        assert!(
            !location.contains("draft_error="),
            "unchanged stale scope must not error: {location}"
        );
        let doc = rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).unwrap();
        assert_eq!(doc.frontmatter.comments[0].body, "Reworded note.");
        assert_eq!(
            doc.frontmatter.comments[0].doc.as_ref().unwrap().stem,
            "phase-1-one",
            "stale scope must survive a body-only edit"
        );
    }

    /// Comment bodies render inside the draft panel's edit textareas:
    /// hostile text must not break out of the textarea or inject script.
    #[tokio::test]
    async fn draft_panel_escapes_textarea_breakout_in_comment_body() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=%3C%2Ftextarea%3E%3Cscript%3Ealert(1)%3C%2Fscript%3E",
            ),
        )
        .await;
        let html =
            get_html_with_cookie(&state, "/projects/demo/tasks/fix-login", "rdm_author=ed").await;
        // Askama escapes with numeric character references.
        assert!(
            html.contains("&#60;/textarea&#62;&#60;script&#62;alert(1)&#60;/script&#62;"),
            "body must render as escaped entities: {html}"
        );
        assert!(
            !html.contains("<script>alert(1)</script>"),
            "no live script injection: {html}"
        );
        assert!(
            !html.contains("</textarea><script>"),
            "no textarea breakout: {html}"
        );
    }

    // -- remove comment --

    #[tokio::test]
    async fn remove_comment_form_removes_and_redirects() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=Keep.",
            ),
        )
        .await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=Drop.",
            ),
        )
        .await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments/2/remove"),
                "",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let doc = rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).unwrap();
        assert_eq!(doc.frontmatter.comments.len(), 1);
        // Remaining ids are not renumbered.
        assert_eq!(doc.frontmatter.comments[0].id, 1);
        assert_eq!(doc.frontmatter.comments[0].body, "Keep.");
    }

    #[tokio::test]
    async fn remove_comment_form_unknown_id_redirects_with_error() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments/9/remove"),
                "",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let location = decoded_location(&response);
        assert!(location.contains("comment 9 not found"), "{location}");
    }

    // -- submit --

    #[tokio::test]
    async fn submit_review_form_missing_verdict_redirects_with_error() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=Note.",
            ),
        )
        .await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/submit"),
                "summary=Looks%20fine.",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let location = decoded_location(&response);
        assert!(location.contains("verdict"), "{location}");
        assert!(location.ends_with("#review-draft"), "{location}");
        let doc = rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).unwrap();
        assert_eq!(doc.frontmatter.state, ReviewState::Draft);
    }

    #[tokio::test]
    async fn submit_review_form_succeeds_and_redirects_to_public_anchor() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments"),
                "body=Note.",
            ),
        )
        .await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/submit"),
                "verdict=request-changes&summary=Overall%20summary.",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        assert_eq!(
            decoded_location(&response),
            format!("/projects/demo/tasks/fix-login#review-{id}")
        );
        let doc = rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).unwrap();
        assert_eq!(doc.frontmatter.state, ReviewState::Submitted);
        assert_eq!(
            doc.frontmatter.verdict,
            Some(rdm_core::model::Verdict::RequestChanges)
        );
        assert_eq!(doc.body, "Overall summary.\n");
    }

    #[tokio::test]
    async fn submit_review_form_blank_summary_keeps_existing_summary() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        // Seed a summary through core, then submit with a blank textarea.
        {
            let mut store = state.store();
            rdm_core::ops::reviews::set_summary(
                &mut store,
                "demo",
                &id,
                rdm_core::ops::BodyUpdate::Set("Existing notes.".to_string()),
            )
            .unwrap();
            rdm_core::store::Store::commit(&mut store).unwrap();
        }
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/submit"),
                "verdict=approve&summary=%20%20",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let doc = rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).unwrap();
        assert_eq!(doc.frontmatter.state, ReviewState::Submitted);
        assert_eq!(doc.body, "Existing notes.\n");
    }

    #[tokio::test]
    async fn submit_review_form_invalid_verdict_redirects_with_error() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/submit"),
                "verdict=bogus&summary=x",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);
        let location = decoded_location(&response);
        assert!(location.contains("invalid verdict"), "{location}");
        assert!(location.contains("approve"), "{location}");
    }

    // -- dismiss --

    #[tokio::test]
    async fn dismiss_review_form_transitions_submitted_to_dismissed() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/submit"),
                "verdict=comment&summary=Notes.",
            ),
        )
        .await;
        let response = send(
            &state,
            post_form(&format!("/projects/demo/reviews/{id}/form/dismiss"), ""),
        )
        .await;
        assert_eq!(response.status(), 303);
        assert_eq!(
            decoded_location(&response),
            format!("/projects/demo/tasks/fix-login#review-{id}")
        );
        let doc = rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).unwrap();
        assert_eq!(doc.frontmatter.state, ReviewState::Dismissed);
    }

    #[tokio::test]
    async fn dismiss_review_form_on_terminal_review_redirects_with_error() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        // Draft → dismissed is a legal transition (abandoning a draft), so
        // dismiss twice: the second hits the terminal-state guard.
        let uri = format!("/projects/demo/reviews/{id}/form/dismiss");
        let response = send(&state, post_form(&uri, "")).await;
        assert_eq!(response.status(), 303);
        let response = send(&state, post_form(&uri, "")).await;
        assert_eq!(response.status(), 303);
        let location = decoded_location(&response);
        assert!(location.contains("cannot move"), "{location}");
        assert!(location.contains("draft_error="), "{location}");
    }

    // -- delete --

    #[tokio::test]
    async fn delete_review_form_removes_draft_and_redirects() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        let response = send(
            &state,
            post_form(&format!("/projects/demo/reviews/{id}/form/delete"), ""),
        )
        .await;
        assert_eq!(response.status(), 303);
        assert_eq!(
            decoded_location(&response),
            "/projects/demo/tasks/fix-login#review-draft"
        );
        assert!(reviews(&state).is_empty());
    }

    #[tokio::test]
    async fn delete_review_form_on_submitted_redirects_with_error() {
        let (_dir, state) = setup();
        let id = start_draft(&state, "task/fix-login", "ed").await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/submit"),
                "verdict=approve&summary=Fine.",
            ),
        )
        .await;
        let response = send(
            &state,
            post_form(&format!("/projects/demo/reviews/{id}/form/delete"), ""),
        )
        .await;
        assert_eq!(response.status(), 303);
        let location = decoded_location(&response);
        assert!(location.contains("not a draft"), "{location}");
        // Still there.
        assert!(rdm_core::ops::reviews::get_review(&state.store(), "demo", &id).is_ok());
    }

    // -- full loop (acceptance criteria sequence, zero JS) --

    #[tokio::test]
    async fn full_form_loop_start_comment_edit_remove_submit() {
        let (_dir, state) = setup();
        // Start on the roadmap page.
        let id = start_draft(&state, "roadmap/alpha", "reviewer").await;
        let comments_uri = format!("/projects/demo/reviews/{id}/form/comments");
        // Add a general (whole-document) comment.
        send(&state, post_form(&comments_uri, "body=General%20remark.")).await;
        // Add a doc-scoped comment targeting the phase.
        send(
            &state,
            post_form(&comments_uri, "body=Phase%20remark.&doc_stem=phase-1-one"),
        )
        .await;
        // Edit the first pending comment.
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments/1/edit"),
                "body=General%20remark%2C%20reworded.&doc_stem=",
            ),
        )
        .await;
        // Add a third and remove it again.
        send(&state, post_form(&comments_uri, "body=Scratch%20that.")).await;
        send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/comments/3/remove"),
                "",
            ),
        )
        .await;
        // Submit with request-changes and a summary.
        let response = send(
            &state,
            post_form(
                &format!("/projects/demo/reviews/{id}/form/submit"),
                "verdict=request-changes&summary=Needs%20work.",
            ),
        )
        .await;
        assert_eq!(response.status(), 303);

        // The file on disk matches: submitted, request-changes, two
        // comments with the edited body and the doc scope intact.
        let store = state.store();
        let doc = rdm_core::ops::reviews::get_review(&store, "demo", &id).unwrap();
        assert_eq!(doc.frontmatter.state, ReviewState::Submitted);
        assert_eq!(
            doc.frontmatter.verdict,
            Some(rdm_core::model::Verdict::RequestChanges)
        );
        assert_eq!(doc.body, "Needs work.\n");
        assert_eq!(doc.frontmatter.comments.len(), 2);
        assert_eq!(
            doc.frontmatter.comments[0].body,
            "General remark, reworded."
        );
        assert!(doc.frontmatter.comments[0].doc.is_none());
        assert_eq!(
            doc.frontmatter.comments[1].doc.as_ref().unwrap().stem,
            "phase-1-one"
        );

        // `rdm review requests` lists submitted request-changes reviews:
        // pin the same filter the CLI applies.
        let requests = rdm_core::ops::reviews::filter_reviews(
            rdm_core::ops::reviews::list_reviews(&store, "demo").unwrap(),
            &rdm_core::ops::reviews::ReviewFilter {
                state: Some(ReviewState::Submitted),
                verdict: Some(rdm_core::model::Verdict::RequestChanges),
                ..Default::default()
            },
        );
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, id);
    }
}
