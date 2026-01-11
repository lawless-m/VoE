//! Web interface for iSCSI target management
//!
//! Provides a REST API and web UI for managing iSCSI targets.

use anyhow::Result;
use axum::{
    extract::{Path, State},
    response::{Html, Json},
    routing::{get, post, delete},
    Router,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;

use aoe_server::iscsi::{CloneManager, TargetRegistry};

#[derive(Parser)]
#[command(name = "iscsi-web")]
#[command(about = "Web interface for iSCSI target management")]
struct Cli {
    /// Bind address for web server
    #[arg(long, default_value = "0.0.0.0:8080")]
    bind: String,

    /// Path to registry file
    #[arg(long, default_value = "/var/lib/voe-iscsi/registry.json")]
    registry: PathBuf,

    /// Base directory for target indexes
    #[arg(long, default_value = "/var/lib/voe-iscsi/targets")]
    targets_dir: PathBuf,

    /// CAS server address
    #[arg(long, default_value = "127.0.0.1:3000")]
    cas_server: String,
}

/// Shared application state
#[derive(Clone)]
struct AppState {
    registry_path: PathBuf,
    targets_dir: PathBuf,
    cas_server: String,
}

impl AppState {
    fn new_manager(&self) -> Result<CloneManager> {
        CloneManager::new(
            self.registry_path.clone(),
            self.targets_dir.clone(),
            self.cas_server.clone(),
        )
    }
}

// API request/response types
#[derive(Deserialize)]
struct CreateTargetRequest {
    name: String,
    size_mb: u64,
    description: Option<String>,
}

#[derive(Deserialize)]
struct CloneTargetRequest {
    source_iqn: String,
    dest_name: String,
}

#[derive(Serialize)]
struct TargetInfo {
    iqn: String,
    name: String,
    size_mb: u64,
    index_path: String,
    parent: Option<String>,
    children: Vec<String>,
    created_at: u64,
    description: Option<String>,
    running: bool,
}

#[derive(Serialize)]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    error: Option<String>,
}

impl<T> ApiResponse<T> {
    fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    fn error(message: String) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();

    let state = AppState {
        registry_path: cli.registry.clone(),
        targets_dir: cli.targets_dir.clone(),
        cas_server: cli.cas_server.clone(),
    };

    // Build router
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/targets", get(list_targets))
        .route("/api/targets", post(create_target))
        .route("/api/targets/{iqn}", get(get_target))
        .route("/api/targets/{iqn}", delete(delete_target))
        .route("/api/targets/clone", post(clone_target))
        .route("/api/targets/{iqn}/gc", post(gc_target))
        .with_state(state);

    let addr: SocketAddr = cli.bind.parse()?;
    println!("iSCSI Web Interface starting on http://{}", addr);
    println!("  Registry: {:?}", cli.registry);
    println!("  Targets:  {:?}", cli.targets_dir);
    println!("  CAS:      {}", cli.cas_server);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Serve the main HTML page
async fn index_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// List all targets
async fn list_targets(State(state): State<AppState>) -> Json<ApiResponse<Vec<TargetInfo>>> {
    match state.new_manager() {
        Ok(manager) => {
            let targets: Vec<TargetInfo> = manager
                .registry
                .list_targets()
                .into_iter()
                .map(|t| {
                    let running = manager.is_target_running(&t.iqn).unwrap_or(false);
                    TargetInfo {
                        iqn: t.iqn.clone(),
                        name: t.name.clone(),
                        size_mb: t.size_mb,
                        index_path: t.index_path.to_string_lossy().to_string(),
                        parent: t.parent.clone(),
                        children: t.children.clone(),
                        created_at: t.created_at,
                        description: t.description.clone(),
                        running,
                    }
                })
                .collect();
            Json(ApiResponse::success(targets))
        }
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

/// Get single target info
async fn get_target(
    State(state): State<AppState>,
    Path(iqn): Path<String>,
) -> Json<ApiResponse<TargetInfo>> {
    match state.new_manager() {
        Ok(manager) => match manager.registry.get_target(&iqn) {
            Some(t) => {
                let running = manager.is_target_running(&t.iqn).unwrap_or(false);
                let info = TargetInfo {
                    iqn: t.iqn.clone(),
                    name: t.name.clone(),
                    size_mb: t.size_mb,
                    index_path: t.index_path.to_string_lossy().to_string(),
                    parent: t.parent.clone(),
                    children: t.children.clone(),
                    created_at: t.created_at,
                    description: t.description.clone(),
                    running,
                };
                Json(ApiResponse::success(info))
            }
            None => Json(ApiResponse::error(format!("Target not found: {}", iqn))),
        },
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

/// Create new target
async fn create_target(
    State(state): State<AppState>,
    Json(req): Json<CreateTargetRequest>,
) -> Json<ApiResponse<String>> {
    match state.new_manager() {
        Ok(mut manager) => {
            match manager.create_target(&req.name, req.size_mb, req.description) {
                Ok(iqn) => Json(ApiResponse::success(iqn)),
                Err(e) => Json(ApiResponse::error(e.to_string())),
            }
        }
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

/// Clone target
async fn clone_target(
    State(state): State<AppState>,
    Json(req): Json<CloneTargetRequest>,
) -> Json<ApiResponse<String>> {
    match state.new_manager() {
        Ok(mut manager) => match manager.clone_target(&req.source_iqn, &req.dest_name) {
            Ok(iqn) => Json(ApiResponse::success(iqn)),
            Err(e) => Json(ApiResponse::error(e.to_string())),
        },
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

/// Delete target
async fn delete_target(
    State(state): State<AppState>,
    Path(iqn): Path<String>,
) -> Json<ApiResponse<String>> {
    match state.new_manager() {
        Ok(mut manager) => match manager.delete_target(&iqn, false) {
            Ok(_) => Json(ApiResponse::success(format!("Deleted target: {}", iqn))),
            Err(e) => Json(ApiResponse::error(e.to_string())),
        },
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

/// Garbage collect target
async fn gc_target(
    State(state): State<AppState>,
    Path(iqn): Path<String>,
) -> Json<ApiResponse<String>> {
    // TODO: Implement GC via async task
    Json(ApiResponse::error(
        "Garbage collection not yet implemented in web interface".to_string(),
    ))
}

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>iSCSI Target Management</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: monospace;
            background: #fafafa;
            padding: 20px;
        }
        .container { max-width: 1200px; margin: 0 auto; }
        h1 {
            margin-bottom: 20px;
            font-size: 24px;
            border-bottom: 2px solid #000;
            padding-bottom: 10px;
        }
        .actions {
            background: #fff;
            padding: 15px;
            border: 1px solid #ddd;
            margin-bottom: 20px;
        }
        .btn {
            background: #000;
            color: #fff;
            border: 1px solid #000;
            padding: 8px 16px;
            cursor: pointer;
            font-size: 13px;
            font-family: monospace;
            margin-right: 10px;
        }
        .btn:hover { background: #333; }
        .btn-danger { background: #fff; color: #d00; border-color: #d00; }
        .btn-danger:hover { background: #d00; color: #fff; }
        .btn-success { background: #fff; color: #080; border-color: #080; }
        .btn-success:hover { background: #080; color: #fff; }
        .targets {
            background: #fff;
            padding: 15px;
            border: 1px solid #ddd;
        }
        .target {
            border-bottom: 1px solid #eee;
            padding: 12px 0;
        }
        .target:last-child { border-bottom: none; }
        .target-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 8px;
        }
        .target-name {
            font-size: 16px;
            font-weight: bold;
        }
        .target-iqn {
            font-size: 11px;
            color: #666;
        }
        .target-info {
            font-size: 12px;
            color: #666;
            margin-top: 4px;
        }
        .badge {
            display: inline-block;
            padding: 2px 6px;
            font-size: 10px;
            font-weight: bold;
            margin-left: 8px;
            border: 1px solid;
        }
        .badge-running { background: #080; color: #fff; border-color: #080; }
        .badge-stopped { background: #666; color: #fff; border-color: #666; }
        .badge-clone { background: #05a; color: #fff; border-color: #05a; }
        .modal {
            display: none;
            position: fixed;
            top: 0;
            left: 0;
            width: 100%;
            height: 100%;
            background: rgba(0,0,0,0.7);
            align-items: center;
            justify-content: center;
        }
        .modal.active { display: flex; }
        .modal-content {
            background: #fff;
            padding: 25px;
            border: 2px solid #000;
            max-width: 500px;
            width: 90%;
        }
        .modal h2 { margin-bottom: 20px; font-size: 18px; }
        .form-group {
            margin-bottom: 15px;
        }
        .form-group label {
            display: block;
            margin-bottom: 5px;
            font-weight: bold;
        }
        .form-group input, .form-group select {
            width: 100%;
            padding: 6px;
            border: 1px solid #000;
            font-size: 13px;
            font-family: monospace;
        }
        .error {
            background: #fee;
            color: #d00;
            padding: 10px;
            border: 1px solid #d00;
            margin-bottom: 15px;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>iSCSI Target Management</h1>

        <div class="actions">
            <button class="btn btn-success" onclick="showCreateModal()">Create Target</button>
            <button class="btn" onclick="showCloneModal()">Clone Target</button>
            <button class="btn" onclick="loadTargets()">Refresh</button>
        </div>

        <div class="targets" id="targets">
            Loading targets...
        </div>
    </div>

    <!-- Create Target Modal -->
    <div class="modal" id="createModal">
        <div class="modal-content">
            <h2>Create New Target</h2>
            <div id="createError" class="error" style="display:none;"></div>
            <div class="form-group">
                <label>Target Name:</label>
                <input type="text" id="createName" placeholder="e.g., DEBIAN-BASE">
            </div>
            <div class="form-group">
                <label>Size (MB):</label>
                <input type="number" id="createSize" value="10240">
            </div>
            <div class="form-group">
                <label>Description (optional):</label>
                <input type="text" id="createDesc" placeholder="e.g., Debian base image">
            </div>
            <button class="btn btn-success" onclick="createTarget()">Create</button>
            <button class="btn" onclick="hideModal('createModal')">Cancel</button>
        </div>
    </div>

    <!-- Clone Target Modal -->
    <div class="modal" id="cloneModal">
        <div class="modal-content">
            <h2>Clone Target</h2>
            <div id="cloneError" class="error" style="display:none;"></div>
            <div class="form-group">
                <label>Source Target:</label>
                <select id="cloneSource"></select>
            </div>
            <div class="form-group">
                <label>Destination Name:</label>
                <input type="text" id="cloneDest" placeholder="e.g., DEBIAN-LIVE1">
            </div>
            <button class="btn btn-success" onclick="cloneTarget()">Clone</button>
            <button class="btn" onclick="hideModal('cloneModal')">Cancel</button>
        </div>
    </div>

    <script>
        let targets = [];

        async function loadTargets() {
            try {
                const res = await fetch('/api/targets');
                const data = await res.json();

                if (data.success) {
                    targets = data.data;
                    renderTargets();
                } else {
                    document.getElementById('targets').innerHTML =
                        `<div class="error">Error loading targets: ${data.error}</div>`;
                }
            } catch (e) {
                document.getElementById('targets').innerHTML =
                    `<div class="error">Failed to load targets: ${e.message}</div>`;
            }
        }

        function renderTargets() {
            const container = document.getElementById('targets');

            if (targets.length === 0) {
                container.innerHTML = '<p>No targets configured.</p>';
                return;
            }

            // Build tree structure
            const roots = targets.filter(t => !t.parent);
            let html = '';

            roots.forEach(root => {
                html += renderTarget(root, 0);
            });

            container.innerHTML = html;
        }

        function renderTarget(target, level) {
            const runningBadge = target.running
                ? '<span class="badge badge-running">RUNNING</span>'
                : '<span class="badge badge-stopped">STOPPED</span>';

            const cloneBadge = target.parent
                ? '<span class="badge badge-clone">CLONE</span>'
                : '';

            let html = `
                <div class="target" style="margin-left: ${level * 30}px">
                    <div class="target-header">
                        <div>
                            <div class="target-name">
                                ${target.name}
                                ${runningBadge}
                                ${cloneBadge}
                            </div>
                            <div class="target-iqn">${target.iqn}</div>
                        </div>
                        <div>
                            ${!target.running ?
                                `<button class="btn btn-danger" onclick="deleteTarget('${target.iqn}')">Delete</button>`
                                : ''}
                        </div>
                    </div>
                    <div class="target-info">
                        Size: ${target.size_mb} MB |
                        ${target.description || 'No description'}
                    </div>
                </div>
            `;

            // Render children
            const children = targets.filter(t => t.parent === target.iqn);
            children.forEach(child => {
                html += renderTarget(child, level + 1);
            });

            return html;
        }

        async function createTarget() {
            const name = document.getElementById('createName').value;
            const size = parseInt(document.getElementById('createSize').value);
            const desc = document.getElementById('createDesc').value;

            try {
                const res = await fetch('/api/targets', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                        name: name,
                        size_mb: size,
                        description: desc || null
                    })
                });

                const data = await res.json();

                if (data.success) {
                    hideModal('createModal');
                    loadTargets();
                } else {
                    showError('createError', data.error);
                }
            } catch (e) {
                showError('createError', e.message);
            }
        }

        async function cloneTarget() {
            const sourceIqn = document.getElementById('cloneSource').value;
            const destName = document.getElementById('cloneDest').value;

            try {
                const res = await fetch('/api/targets/clone', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                        source_iqn: sourceIqn,
                        dest_name: destName
                    })
                });

                const data = await res.json();

                if (data.success) {
                    hideModal('cloneModal');
                    loadTargets();
                } else {
                    showError('cloneError', data.error);
                }
            } catch (e) {
                showError('cloneError', e.message);
            }
        }

        async function deleteTarget(iqn) {
            if (!confirm(`Delete target ${iqn}?`)) return;

            try {
                const res = await fetch(`/api/targets/${encodeURIComponent(iqn)}`, {
                    method: 'DELETE'
                });

                const data = await res.json();

                if (data.success) {
                    loadTargets();
                } else {
                    alert('Error: ' + data.error);
                }
            } catch (e) {
                alert('Failed to delete: ' + e.message);
            }
        }

        function showCreateModal() {
            document.getElementById('createModal').classList.add('active');
        }

        function showCloneModal() {
            const select = document.getElementById('cloneSource');
            select.innerHTML = targets
                .filter(t => !t.running)
                .map(t => `<option value="${t.iqn}">${t.name}</option>`)
                .join('');
            document.getElementById('cloneModal').classList.add('active');
        }

        function hideModal(id) {
            document.getElementById(id).classList.remove('active');
            document.querySelectorAll('.error').forEach(e => e.style.display = 'none');
        }

        function showError(id, msg) {
            const el = document.getElementById(id);
            el.textContent = msg;
            el.style.display = 'block';
        }

        // Load targets on page load
        loadTargets();
    </script>
</body>
</html>
"#;
