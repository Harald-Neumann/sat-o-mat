use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use serde::Serialize;
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::config::Permission;
use chrono::{DateTime, Utc};

use crate::task::format::{TASK_STATES, Task};
use crate::task::utils::check_time_conflict;

use super::AppState;
use super::auth::AuthenticatedKey;
use super::error::ApiError;

const EDITABLE_STATES: &[&str] = &["Active", "PendingApproval"];

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskListEntry {
    pub id: String,
    pub state: String,
    pub start: Option<String>,
    pub end: Option<String>,
}

/// List all tasks.
///
/// Returns tasks in all states (Active, PendingApproval, Completed and Failed).
#[utoipa::path(
    get,
    path = "/tasks",
    tag = super::TASKS_TAG,
    responses(
        (status = 200, description = "List of tasks", body = Vec<TaskListEntry>),
        (status = 401, description = "Missing or invalid API key"),
        (status = 403, description = "Insufficient permissions"),
    ),
    security(("api_key" = []))
)]
pub async fn list_tasks(
    State(state): State<AppState>,
    auth: AuthenticatedKey,
) -> Result<Json<Vec<TaskListEntry>>, ApiError> {
    auth.require(Permission::ViewTasks)?;

    let mut entries = Vec::new();
    for &dir in TASK_STATES {
        let dir_path = state.tasks_path.join(dir);
        let Ok(mut read_dir) = tokio::fs::read_dir(&dir_path).await else {
            continue;
        };
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let file_name = entry.file_name().to_string_lossy().to_string();
            let id = Task::id_from_filename(&file_name).to_string();

            let Some(task) = tokio::fs::read_to_string(entry.path())
                .await
                .ok()
                .and_then(|c| Task::from_yaml_str(&c).ok())
            else {
                continue;
            };

            let start = task.get_time_variable("start").ok();
            let end = task.get_time_variable("end").ok();

            entries.push((
                start,
                TaskListEntry {
                    id,
                    state: dir.to_string(),
                    start: start.map(|t| t.to_string()),
                    end: end.map(|t| t.to_string()),
                },
            ));
        }
    }

    entries.sort_by(|a, b| b.0.cmp(&a.0));

    Ok(Json(entries.into_iter().map(|(_, e)| e).collect()))
}

/// Get the full YAML text of a specific task.
#[utoipa::path(
    get,
    path = "/tasks/{id}",
    tag = super::TASKS_TAG,
    params(
        ("id" = String, Path, description = "Task unique identifier (filename)")
    ),
    responses(
        (status = 200, description = "Task YAML content", body = String),
        (status = 401, description = "Missing or invalid API key"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    ),
    security(("api_key" = []))
)]
pub async fn get_task(
    State(state): State<AppState>,
    auth: AuthenticatedKey,
    AxumPath(id): AxumPath<String>,
) -> Result<String, ApiError> {
    auth.require(Permission::ViewTasks)?;

    let (_task_state, content) = Task::find(&state.tasks_path, &id)
        .await
        .ok_or(ApiError::NotFound)?;

    Ok(content)
}

/// Create or update a task.
///
/// If the task does not exist, requires SubmitTask permission. New tasks are placed in
/// PendingApproval unless the API key has AutoApproveTask permission, in which case
/// they go directly to Active.
///
/// If the task already exists, requires EditTask permission. Only tasks in Active or
/// PendingApproval state can be edited.
///
/// Returns 409 if the task's time range conflicts with another active task.
#[utoipa::path(
    put,
    path = "/tasks/{id}",
    tag = super::TASKS_TAG,
    params(
        ("id" = String, Path, description = "Task unique identifier (filename)")
    ),
    request_body = String,
    responses(
        (status = 200, description = "Task updated"),
        (status = 201, description = "Task created"),
        (status = 400, description = "Invalid task definition"),
        (status = 401, description = "Missing or invalid API key"),
        (status = 403, description = "Insufficient permissions"),
        (status = 409, description = "Task is not editable or has a time conflict"),
    ),
    security(("api_key" = []))
)]
pub async fn put_task(
    State(state): State<AppState>,
    auth: AuthenticatedKey,
    AxumPath(id): AxumPath<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    // Reject path traversal
    if id.contains('/') || id.contains('\\') || id == ".." || id == "." {
        return Err(ApiError::BadRequest("invalid task ID".to_string()));
    }

    // Validate the task definition
    let task = Task::from_yaml_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // Check for time conflicts with other active tasks
    if let Some(conflict) = check_time_conflict(&state.tasks_path, &id, &task).await {
        return Err(ApiError::Conflict(format!(
            "time conflict with task '{conflict}'"
        )));
    }

    let (target_dir, status) = match Task::find(&state.tasks_path, &id).await {
        Some((task_state, _)) => {
            auth.require(Permission::EditTask)?;
            if !EDITABLE_STATES.contains(&task_state.as_str()) {
                return Err(ApiError::Conflict(format!(
                    "task in state '{task_state}' cannot be edited"
                )));
            }
            (task_state, StatusCode::OK)
        }
        None => {
            auth.require(Permission::SubmitTask)?;
            let dir = if auth.has(Permission::AutoApproveTask) {
                "Active"
            } else {
                "PendingApproval"
            };
            (dir.to_string(), StatusCode::CREATED)
        }
    };

    let file_path = state.tasks_path.join(&target_dir).join(Task::filename(&id));
    tokio::fs::write(&file_path, &body).await.map_err(|e| {
        warn!(%id, ?e, "failed to write task file");
        ApiError::Internal
    })?;

    info!(%id, %target_dir, created = status == StatusCode::CREATED);
    Ok(status)
}

/// Delete a task
///
/// Only tasks in Active or PendingApproval state can be deleted.
#[utoipa::path(
    delete,
    path = "/tasks/{id}",
    tag = super::TASKS_TAG,
    params(
        ("id" = String, Path, description = "Task unique identifier (filename)")
    ),
    responses(
        (status = 204, description = "Task deleted"),
        (status = 401, description = "Missing or invalid API key"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
        (status = 409, description = "Task is not in a deletable state"),
    ),
    security(("api_key" = []))
)]
pub async fn delete_task(
    State(state): State<AppState>,
    auth: AuthenticatedKey,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    auth.require(Permission::DeleteTask)?;

    let (task_state, _) = Task::find(&state.tasks_path, &id)
        .await
        .ok_or(ApiError::NotFound)?;

    if !EDITABLE_STATES.contains(&task_state.as_str()) {
        return Err(ApiError::Conflict(format!(
            "task in state '{task_state}' cannot be deleted"
        )));
    }

    let file_path = state.tasks_path.join(&task_state).join(Task::filename(&id));
    tokio::fs::remove_file(&file_path).await.map_err(|e| {
        warn!(%id, ?e, "failed to delete task file");
        ApiError::Internal
    })?;

    info!(%id, %task_state, "task deleted");
    Ok(StatusCode::NO_CONTENT)
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

    const TASK_YAML: &str = "\
variables:
  start: \"2026-06-01T10:00:00Z\"
  end: \"2026-06-01T10:30:00Z\"
steps:
  - cmd: \"echo hello\"
    wait: true
";

    fn task_yaml_at(start: &str, end: &str) -> String {
        format!(
            "variables:\n  start: \"{start}\"\n  end: \"{end}\"\nsteps:\n  - cmd: \"true\"\n    wait: true\n"
        )
    }

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

    fn all_permissions() -> Vec<Permission> {
        vec![
            Permission::ViewTasks,
            Permission::SubmitTask,
            Permission::EditTask,
            Permission::DeleteTask,
            Permission::AutoApproveTask,
        ]
    }

    fn setup(permissions: Vec<Permission>) -> (TempDir, axum::Router) {
        let tmp = tempfile::tempdir().unwrap();
        for dir in ["Active", "PendingApproval", "Completed", "Failed"] {
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

    // --- Auth tests ---

    #[tokio::test]
    async fn missing_api_key_returns_401() {
        let (_, router) = setup(all_permissions());
        let req = Request::get("/api/tasks").body(Body::empty()).unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn invalid_api_key_returns_401() {
        let (_, router) = setup(all_permissions());
        let req = Request::get("/api/tasks")
            .header("api_key", "wrong-key")
            .body(Body::empty())
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn insufficient_permissions_returns_403() {
        let (_, router) = setup(vec![Permission::SubmitTask]); // no ViewTasks
        let req = Request::get("/api/tasks")
            .header("api_key", "test-key")
            .body(Body::empty())
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::FORBIDDEN);
    }

    // --- List tests ---

    #[tokio::test]
    async fn list_empty() {
        let (_, router) = setup(all_permissions());
        let (status, body) = response_body(
            router,
            Request::get("/api/tasks")
                .header("api_key", "test-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "[]");
    }

    #[tokio::test]
    async fn list_returns_tasks_across_states() {
        let (tmp, router) = setup(all_permissions());
        std::fs::write(tmp.path().join("Active/a.yaml"), TASK_YAML).unwrap();
        std::fs::write(tmp.path().join("Completed/b.yaml"), TASK_YAML).unwrap();

        let (status, body) = response_body(
            router,
            Request::get("/api/tasks")
                .header("api_key", "test-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // Two entries: each has an "id" field
        assert_eq!(body.matches("\"id\"").count(), 2);
    }

    // --- Get tests ---

    #[tokio::test]
    async fn get_existing_task() {
        let (tmp, router) = setup(all_permissions());
        std::fs::write(tmp.path().join("Active/t.yaml"), TASK_YAML).unwrap();

        let (status, body) = response_body(
            router,
            Request::get("/api/tasks/t")
                .header("api_key", "test-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, TASK_YAML);
    }

    #[tokio::test]
    async fn get_nonexistent_returns_404() {
        let (_, router) = setup(all_permissions());
        let req = Request::get("/api/tasks/nope")
            .header("api_key", "test-key")
            .body(Body::empty())
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::NOT_FOUND);
    }

    // --- Put (create) tests ---

    #[tokio::test]
    async fn put_creates_new_task_in_pending_approval() {
        let (tmp, router) = setup(vec![Permission::ViewTasks, Permission::SubmitTask]);
        let req = Request::put("/api/tasks/new")
            .header("api_key", "test-key")
            .body(Body::from(TASK_YAML))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::CREATED);
        assert!(tmp.path().join("PendingApproval/new.yaml").exists());
    }

    #[tokio::test]
    async fn put_creates_in_active_with_auto_approve() {
        let (tmp, router) = setup(vec![Permission::SubmitTask, Permission::AutoApproveTask]);
        let req = Request::put("/api/tasks/new")
            .header("api_key", "test-key")
            .body(Body::from(TASK_YAML))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::CREATED);
        assert!(tmp.path().join("Active/new.yaml").exists());
    }

    #[tokio::test]
    async fn put_create_without_submit_permission_returns_403() {
        let (_, router) = setup(vec![Permission::ViewTasks]);
        let req = Request::put("/api/tasks/new")
            .header("api_key", "test-key")
            .body(Body::from(TASK_YAML))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn put_invalid_yaml_returns_400() {
        let (_, router) = setup(all_permissions());
        let req = Request::put("/api/tasks/bad")
            .header("api_key", "test-key")
            .body(Body::from("not: valid: yaml: ["))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_invalid_task_definition_returns_400() {
        let (_, router) = setup(all_permissions());
        let yaml = "variables:\n  start: \"2026-01-01T00:00:00Z\"\nsteps: foobar\n";
        let req = Request::put("/api/tasks/bad")
            .header("api_key", "test-key")
            .body(Body::from(yaml))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::BAD_REQUEST);
    }

    // --- Put (update) tests ---

    #[tokio::test]
    async fn put_updates_existing_active_task() {
        let (tmp, router) = setup(all_permissions());
        std::fs::write(tmp.path().join("Active/t.yaml"), TASK_YAML).unwrap();

        let updated = task_yaml_at("2026-06-01T11:00:00Z", "2026-06-01T11:30:00Z");
        let req = Request::put("/api/tasks/t")
            .header("api_key", "test-key")
            .body(Body::from(updated.clone()))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::OK);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("Active/t.yaml")).unwrap(),
            updated
        );
    }

    #[tokio::test]
    async fn put_update_without_edit_permission_returns_403() {
        let (tmp, router) = setup(vec![Permission::ViewTasks, Permission::SubmitTask]);
        std::fs::write(tmp.path().join("Active/t.yaml"), TASK_YAML).unwrap();

        let req = Request::put("/api/tasks/t")
            .header("api_key", "test-key")
            .body(Body::from(TASK_YAML))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn put_completed_task_returns_409() {
        let (tmp, router) = setup(all_permissions());
        std::fs::write(tmp.path().join("Completed/t.yaml"), TASK_YAML).unwrap();

        let req = Request::put("/api/tasks/t")
            .header("api_key", "test-key")
            .body(Body::from(TASK_YAML))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::CONFLICT);
    }

    // --- Put (time conflict) tests ---

    #[tokio::test]
    async fn put_overlapping_task_returns_409() {
        let (tmp, router) = setup(all_permissions());
        // Existing active task: 10:00 - 10:30
        std::fs::write(
            tmp.path().join("Active/existing.yaml"),
            task_yaml_at("2026-06-01T10:00:00Z", "2026-06-01T10:30:00Z"),
        )
        .unwrap();

        // New task overlaps: 10:15 - 10:45
        let req = Request::put("/api/tasks/new")
            .header("api_key", "test-key")
            .body(Body::from(task_yaml_at(
                "2026-06-01T10:15:00Z",
                "2026-06-01T10:45:00Z",
            )))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn put_non_overlapping_task_succeeds() {
        let (tmp, router) = setup(all_permissions());
        std::fs::write(
            tmp.path().join("Active/existing.yaml"),
            task_yaml_at("2026-06-01T10:00:00Z", "2026-06-01T10:30:00Z"),
        )
        .unwrap();

        // Non-overlapping: 11:00 - 11:30
        let req = Request::put("/api/tasks/new")
            .header("api_key", "test-key")
            .body(Body::from(task_yaml_at(
                "2026-06-01T11:00:00Z",
                "2026-06-01T11:30:00Z",
            )))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn put_update_own_time_range_no_self_conflict() {
        let (tmp, router) = setup(all_permissions());
        std::fs::write(
            tmp.path().join("Active/t.yaml"),
            task_yaml_at("2026-06-01T10:00:00Z", "2026-06-01T10:30:00Z"),
        )
        .unwrap();

        // Update the same task with a slightly shifted range (no other task to conflict with)
        let req = Request::put("/api/tasks/t")
            .header("api_key", "test-key")
            .body(Body::from(task_yaml_at(
                "2026-06-01T10:05:00Z",
                "2026-06-01T10:35:00Z",
            )))
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::OK);
    }

    // --- Delete tests ---

    #[tokio::test]
    async fn delete_active_task() {
        let (tmp, router) = setup(all_permissions());
        std::fs::write(tmp.path().join("Active/t.yaml"), TASK_YAML).unwrap();

        let req = Request::delete("/api/tasks/t")
            .header("api_key", "test-key")
            .body(Body::empty())
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::NO_CONTENT);
        assert!(!tmp.path().join("Active/t.yaml").exists());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_404() {
        let (_, router) = setup(all_permissions());
        let req = Request::delete("/api/tasks/nope")
            .header("api_key", "test-key")
            .body(Body::empty())
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_completed_returns_409() {
        let (tmp, router) = setup(all_permissions());
        std::fs::write(tmp.path().join("Completed/t.yaml"), TASK_YAML).unwrap();

        let req = Request::delete("/api/tasks/t")
            .header("api_key", "test-key")
            .body(Body::empty())
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn delete_without_permission_returns_403() {
        let (tmp, router) = setup(vec![Permission::ViewTasks]);
        std::fs::write(tmp.path().join("Active/t.yaml"), TASK_YAML).unwrap();

        let req = Request::delete("/api/tasks/t")
            .header("api_key", "test-key")
            .body(Body::empty())
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::FORBIDDEN);
    }

    // --- Path traversal ---

    #[tokio::test]
    async fn path_traversal_rejected() {
        let (_, router) = setup(all_permissions());
        let req = Request::get("/api/tasks/..%2F..%2Fetc%2Fpasswd")
            .header("api_key", "test-key")
            .body(Body::empty())
            .unwrap();
        assert_eq!(response_status(router, req).await, StatusCode::NOT_FOUND);
    }
}
