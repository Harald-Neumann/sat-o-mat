use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::config::Permission;
use crate::task::format::Task;
use crate::task::utils::check_time_conflict;

use super::AppState;
use super::auth::AuthenticatedKey;
use super::error::ApiError;

const TEMPLATES_DIR: &str = "Templates";

#[derive(Debug, Serialize, ToSchema)]
pub struct TemplateListEntry {
    pub id: String,
}

/// List available task templates.
#[utoipa::path(
    get,
    path = "/templates",
    tag = super::TEMPLATES_TAG,
    responses(
        (status = 200, description = "List of templates", body = Vec<TemplateListEntry>),
        (status = 401, description = "Missing or invalid API key"),
        (status = 403, description = "Insufficient permissions"),
    ),
    security(("api_key" = []))
)]
pub async fn list_templates(
    State(state): State<AppState>,
    auth: AuthenticatedKey,
) -> Result<Json<Vec<TemplateListEntry>>, ApiError> {
    auth.require(Permission::SubmitFromTemplate)?;

    let dir_path = state.tasks_path.join(TEMPLATES_DIR);
    let Ok(mut read_dir) = tokio::fs::read_dir(&dir_path).await else {
        return Ok(Json(Vec::new()));
    };

    let mut entries = Vec::new();
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let id = Task::id_from_filename(&file_name).to_string();
        entries.push(TemplateListEntry { id });
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(Json(entries))
}

/// Get the full YAML text of a task template.
#[utoipa::path(
    get,
    path = "/templates/{id}",
    tag = super::TEMPLATES_TAG,
    params(
        ("id" = String, Path, description = "Template identifier (filename without extension)")
    ),
    responses(
        (status = 200, description = "Template YAML content", body = String),
        (status = 401, description = "Missing or invalid API key"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Template not found"),
    ),
    security(("api_key" = []))
)]
pub async fn get_template(
    State(state): State<AppState>,
    auth: AuthenticatedKey,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<String, ApiError> {
    auth.require(Permission::SubmitFromTemplate)?;

    read_template(&state, &id).await.map(|(_, content)| content)
}

#[derive(Deserialize, ToSchema)]
pub struct SubmitFromTemplateRequest {
    pub template_id: String,
    pub task_id: String,
    pub variables: HashMap<String, String>,
}

/// Submit a task based on a template.
///
/// Reads the template's steps and cleanup, combines them with the provided
/// variables, and creates a new task. Users can only change variables — the
/// commands are fixed by the template.
///
/// New tasks are placed in PendingApproval unless the API key also has
/// AutoApproveTask permission.
#[utoipa::path(
    post,
    path = "/tasks/submit_from_template",
    tag = super::TEMPLATES_TAG,
    request_body = SubmitFromTemplateRequest,
    responses(
        (status = 201, description = "Task created from template"),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Missing or invalid API key"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Template not found"),
        (status = 409, description = "Task already exists or has a time conflict"),
    ),
    security(("api_key" = []))
)]
pub async fn submit_from_template(
    State(state): State<AppState>,
    auth: AuthenticatedKey,
    Json(req): Json<SubmitFromTemplateRequest>,
) -> Result<StatusCode, ApiError> {
    auth.require(Permission::SubmitFromTemplate)?;

    let task_id = &req.task_id;
    let template_id = &req.template_id;

    // Reject path traversal
    if task_id.contains('/') || task_id.contains('\\') || task_id == ".." || task_id == "." {
        return Err(ApiError::BadRequest("invalid task ID".to_string()));
    }

    // Load the template
    let (template, _) = read_template(&state, template_id).await?;

    // Build the task: template steps + user-provided variables
    let task = Task::new(req.variables, template.steps, template.cleanup);

    // Reject if a task with this ID already exists
    if Task::find(&state.tasks_path, task_id).await.is_some() {
        return Err(ApiError::Conflict(format!(
            "task '{task_id}' already exists"
        )));
    }

    // Check for time conflicts
    if let Some(conflict) = check_time_conflict(&state.tasks_path, task_id, &task).await {
        return Err(ApiError::Conflict(format!(
            "time conflict with task '{conflict}'"
        )));
    }

    let target_dir = if auth.has(Permission::AutoApproveTask) {
        "Active"
    } else {
        "PendingApproval"
    };

    let yaml = serde_yaml::to_string(&task)
        .map_err(|e| ApiError::BadRequest(format!("failed to serialize task: {e}")))?;

    let file_path = state
        .tasks_path
        .join(target_dir)
        .join(Task::filename(task_id));
    tokio::fs::write(&file_path, &yaml).await.map_err(|e| {
        warn!(%task_id, ?e, "failed to write task file");
        ApiError::Internal
    })?;

    info!(%task_id, %template_id, %target_dir, "task created from template");
    Ok(StatusCode::CREATED)
}

/// Read and parse a template file. Returns (parsed Task, raw YAML content).
async fn read_template(state: &AppState, id: &str) -> Result<(Task, String), ApiError> {
    if id.contains('/') || id.contains('\\') || id == ".." || id == "." {
        return Err(ApiError::BadRequest("invalid template ID".to_string()));
    }

    let path = state
        .tasks_path
        .join(TEMPLATES_DIR)
        .join(Task::filename(id));

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| ApiError::NotFound)?;

    let task = Task::from_yaml_str(&content)
        .map_err(|e| ApiError::BadRequest(format!("invalid template: {e}")))?;

    Ok((task, content))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::api;
    use crate::config::{ApiConfig, ApiKey, Config, Permission};

    const TEMPLATE_YAML: &str = "\
variables:
  start: \"2026-06-01T10:00:00Z\"
  end: \"2026-06-01T10:30:00Z\"
steps:
  - cmd: \"echo hello\"
    wait: true
";

    fn test_config(tmp: &TempDir, permissions: Vec<Permission>) -> Config {
        Config {
            station_name: "test".into(),
            api: ApiConfig {
                keys: vec![ApiKey {
                    key: "test-key".into(),
                    permissions,
                }],
            },
            tasks_path: tmp.path().to_path_buf(),
            tle_path: tmp.path().join("tle"),
            ground_station: None,
        }
    }

    fn setup(permissions: Vec<Permission>) -> (TempDir, axum::Router) {
        let tmp = tempfile::tempdir().unwrap();
        for dir in [
            "Active",
            "PendingApproval",
            "Completed",
            "Failed",
            "Templates",
        ] {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        let config = test_config(&tmp, permissions);
        let (router, _) = api::router(&config).split_for_parts();
        (tmp, router)
    }

    async fn response_status(router: axum::Router, req: Request<Body>) -> StatusCode {
        router.oneshot(req).await.unwrap().status()
    }

    async fn response_body(router: axum::Router, req: Request<Body>) -> (StatusCode, String) {
        let resp = router.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    fn submit_json(template_id: &str, task_id: &str, vars: &[(&str, &str)]) -> String {
        let vars_entries: Vec<String> = vars
            .iter()
            .map(|(k, v)| format!("\"{k}\": \"{v}\""))
            .collect();
        format!(
            r#"{{"template_id": "{template_id}", "task_id": "{task_id}", "variables": {{{}}}}}"#,
            vars_entries.join(", ")
        )
    }

    // --- List templates ---

    #[tokio::test]
    async fn list_templates_empty() {
        let (_, router) = setup(vec![Permission::SubmitFromTemplate]);
        let (status, body) = response_body(
            router,
            Request::get("/api/templates")
                .header("api_key", "test-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "[]");
    }

    #[tokio::test]
    async fn list_templates_returns_entries() {
        let (tmp, router) = setup(vec![Permission::SubmitFromTemplate]);
        std::fs::write(tmp.path().join("Templates/passA.yaml"), TEMPLATE_YAML).unwrap();
        std::fs::write(tmp.path().join("Templates/passB.yaml"), TEMPLATE_YAML).unwrap();

        let (status, body) = response_body(
            router,
            Request::get("/api/templates")
                .header("api_key", "test-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.matches("\"id\"").count(), 2);
    }

    #[tokio::test]
    async fn list_templates_without_permission_returns_403() {
        let (_, router) = setup(vec![Permission::ViewTasks]);
        let req = Request::get("/api/templates")
            .header("api_key", "test-key")
            .body(Body::empty())
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::FORBIDDEN);
    }

    // --- Get template ---

    #[tokio::test]
    async fn get_template_returns_yaml() {
        let (tmp, router) = setup(vec![Permission::SubmitFromTemplate]);
        std::fs::write(tmp.path().join("Templates/passA.yaml"), TEMPLATE_YAML).unwrap();

        let (status, body) = response_body(
            router,
            Request::get("/api/templates/passA")
                .header("api_key", "test-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, TEMPLATE_YAML);
    }

    #[tokio::test]
    async fn get_nonexistent_template_returns_404() {
        let (_, router) = setup(vec![Permission::SubmitFromTemplate]);
        let req = Request::get("/api/templates/nope")
            .header("api_key", "test-key")
            .body(Body::empty())
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::NOT_FOUND);
    }

    // --- Submit from template ---

    #[tokio::test]
    async fn submit_from_template_creates_task() {
        let (tmp, router) = setup(vec![Permission::SubmitFromTemplate]);
        std::fs::write(tmp.path().join("Templates/passA.yaml"), TEMPLATE_YAML).unwrap();

        let req = Request::post("/api/tasks/submit_from_template")
            .header("api_key", "test-key")
            .header("content-type", "application/json")
            .body(Body::from(submit_json("passA", "mypass", &[
                ("start", "2026-07-01T10:00:00Z"),
                ("end", "2026-07-01T10:30:00Z"),
            ])))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::CREATED);
        assert!(tmp.path().join("PendingApproval/mypass.yaml").exists());
    }

    #[tokio::test]
    async fn submit_from_template_auto_approve() {
        let (tmp, router) = setup(vec![
            Permission::SubmitFromTemplate,
            Permission::AutoApproveTask,
        ]);
        std::fs::write(tmp.path().join("Templates/passA.yaml"), TEMPLATE_YAML).unwrap();

        let req = Request::post("/api/tasks/submit_from_template")
            .header("api_key", "test-key")
            .header("content-type", "application/json")
            .body(Body::from(submit_json("passA", "mypass", &[
                ("start", "2026-07-01T10:00:00Z"),
                ("end", "2026-07-01T10:30:00Z"),
            ])))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::CREATED);
        assert!(tmp.path().join("Active/mypass.yaml").exists());
    }

    #[tokio::test]
    async fn submit_duplicate_task_id_returns_409() {
        let (tmp, router) = setup(vec![Permission::SubmitFromTemplate]);
        std::fs::write(tmp.path().join("Templates/passA.yaml"), TEMPLATE_YAML).unwrap();
        std::fs::write(tmp.path().join("Active/existing.yaml"), TEMPLATE_YAML).unwrap();

        let req = Request::post("/api/tasks/submit_from_template")
            .header("api_key", "test-key")
            .header("content-type", "application/json")
            .body(Body::from(submit_json("passA", "existing", &[
                ("start", "2026-07-01T10:00:00Z"),
                ("end", "2026-07-01T10:30:00Z"),
            ])))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn submit_nonexistent_template_returns_404() {
        let (_, router) = setup(vec![Permission::SubmitFromTemplate]);
        let req = Request::post("/api/tasks/submit_from_template")
            .header("api_key", "test-key")
            .header("content-type", "application/json")
            .body(Body::from(submit_json("nope", "task1", &[
                ("start", "2026-07-01T10:00:00Z"),
                ("end", "2026-07-01T10:30:00Z"),
            ])))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn submit_time_conflict_returns_409() {
        let (tmp, router) = setup(vec![
            Permission::SubmitFromTemplate,
            Permission::AutoApproveTask,
        ]);
        std::fs::write(tmp.path().join("Templates/passA.yaml"), TEMPLATE_YAML).unwrap();
        std::fs::write(tmp.path().join("Active/other.yaml"), TEMPLATE_YAML).unwrap();

        let req = Request::post("/api/tasks/submit_from_template")
            .header("api_key", "test-key")
            .header("content-type", "application/json")
            .body(Body::from(submit_json("passA", "conflict", &[
                ("start", "2026-06-01T10:00:00Z"),
                ("end", "2026-06-01T10:30:00Z"),
            ])))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn submit_without_permission_returns_403() {
        let (tmp, router) = setup(vec![Permission::ViewTasks]);
        std::fs::write(tmp.path().join("Templates/passA.yaml"), TEMPLATE_YAML).unwrap();

        let req = Request::post("/api/tasks/submit_from_template")
            .header("api_key", "test-key")
            .header("content-type", "application/json")
            .body(Body::from(submit_json("passA", "task1", &[
                ("start", "2026-07-01T10:00:00Z"),
                ("end", "2026-07-01T10:30:00Z"),
            ])))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::FORBIDDEN);
    }
}
