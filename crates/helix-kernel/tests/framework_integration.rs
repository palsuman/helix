mod support;

use helix_ipc::{PING, PingRequest, PingResponse};
use helix_state::commands::{
    LAYOUT_GET, LAYOUT_SET, LayoutGetRequest, LayoutGetResponse, LayoutSetRequest,
    LayoutSetResponse,
};
use helix_workspace::commands::{OPEN, WorkspaceOpenRequest, WorkspaceResponse};
use support::{KernelHarness, StreamClient, TestWorkspace};

#[tokio::test]
async fn real_container_serves_typed_ipc_after_restart() {
    let workspace = TestWorkspace::new();
    workspace.write("src/example.txt", "original");
    workspace.assert_file("src/example.txt", "original");
    let mut kernel = KernelHarness::start(workspace).await;
    let response: PingResponse = kernel
        .command(
            PING,
            PingRequest {
                message: "hello".into(),
            },
        )
        .await;
    assert_eq!(response.echo, "hello");
    let epoch = kernel.epoch.clone();
    kernel.crash_and_restart().await;
    assert_ne!(kernel.epoch, epoch);
    let response: PingResponse = kernel
        .command(
            PING,
            PingRequest {
                message: "restarted".into(),
            },
        )
        .await;
    assert_eq!(response.echo, "restarted");
    kernel.shutdown().await;
}

#[tokio::test]
async fn workspace_driven_over_ipc_recovers_persisted_state_after_force_kill() {
    let workspace = TestWorkspace::new();
    workspace.write("src/example.txt", "original");
    let root = workspace.root().to_string_lossy().into_owned();
    let mut kernel = KernelHarness::start(workspace).await;
    let mut stream = StreamClient::subscribe(&kernel, helix_workspace::commands::CHANNEL).await;
    let opened: WorkspaceResponse = kernel
        .command(
            OPEN,
            WorkspaceOpenRequest {
                roots: vec![root.clone()],
                name: None,
            },
        )
        .await;
    let messages = stream.collect_ordered(1).await;
    assert_eq!(messages.len(), 1);
    let key = opened.workspace.key;
    let layout = serde_json::json!({ "panelSize": 321, "activePanel": "test" });
    let saved: LayoutSetResponse = kernel
        .command(
            LAYOUT_SET,
            LayoutSetRequest {
                workspace_key: key.clone(),
                layout: layout.clone(),
            },
        )
        .await;
    assert!(saved.persisted);
    drop(stream);
    kernel.crash_and_restart().await;
    let reopened: WorkspaceResponse = kernel
        .command(
            OPEN,
            WorkspaceOpenRequest {
                roots: vec![root],
                name: None,
            },
        )
        .await;
    assert_eq!(reopened.workspace.key, key);
    let restored: LayoutGetResponse = kernel
        .command(LAYOUT_GET, LayoutGetRequest { workspace_key: key })
        .await;
    assert_eq!(restored.layout, layout);
    assert_eq!(restored.projection_hash, saved.projection_hash);
    kernel.shutdown().await;
}

#[tokio::test]
async fn websocket_client_collects_contiguous_channel_messages() {
    let kernel = KernelHarness::start(TestWorkspace::new()).await;
    let mut stream = StreamClient::subscribe(&kernel, "demo:counter").await;
    assert_eq!(stream.collect_ordered(10).await.len(), 10);
    assert_eq!(stream.collect_ordered(10).await.len(), 10);
    drop(stream);
    kernel.shutdown().await;
}

#[tokio::test]
async fn real_services_delegate_to_a_mock_external_process() {
    use helix_trust::{
        TrustDecision,
        commands::{SET, TrustSetRequest, TrustSetResponse},
    };
    use helix_workspace::commands::{
        AFFECTED_PROJECTS, AffectedProjectsRequest, AffectedProjectsResponse,
        AffectedProjectsSource, PROJECT_GRAPH, ProjectGraphRequest, ProjectGraphResponse,
    };

    let workspace = TestWorkspace::new();
    workspace.mock_nx();
    let changed = workspace.write("apps/fixture/example.txt", "changed");
    let root = workspace.root().to_string_lossy().into_owned();
    let kernel = KernelHarness::start(workspace).await;
    let trusted: TrustSetResponse = kernel
        .command(
            SET,
            TrustSetRequest {
                path: root.clone(),
                decision: TrustDecision::Trusted,
                inherit_to_children: true,
            },
        )
        .await;
    assert!(trusted.applied);
    let opened: WorkspaceResponse = kernel
        .command(
            OPEN,
            WorkspaceOpenRequest {
                roots: vec![root],
                name: None,
            },
        )
        .await;
    let key = opened.workspace.key;
    tokio::time::timeout(support::TIMEOUT, async {
        loop {
            let response: ProjectGraphResponse = kernel
                .command(PROJECT_GRAPH, ProjectGraphRequest { key: key.clone() })
                .await;
            if response
                .graph
                .projects
                .iter()
                .any(|project| project.name == "fixture-app")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("fixture project graph must become ready");
    let affected: AffectedProjectsResponse = kernel
        .command(
            AFFECTED_PROJECTS,
            AffectedProjectsRequest {
                key,
                changed_files: vec![changed.to_string_lossy().into_owned()],
            },
        )
        .await;
    assert_eq!(affected.source, AffectedProjectsSource::Tool);
    assert_eq!(affected.projects.len(), 1);
    assert_eq!(affected.projects[0].name, "fixture-app");
    kernel.assert_mock_nx_called();
    kernel.shutdown().await;
}
