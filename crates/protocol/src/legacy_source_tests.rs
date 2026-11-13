use super::*;
use nexum_core::nexum_scheduler::SchedulerConfig;
use nexum_core::nexum_storage::{StoredTask, TaskRepository};
use nexum_task::{DownloadTask, TaskState};
use serde_json::json;

#[test]
fn legacy_create_rejects_non_http_without_storage_or_events() {
    let mut core = Core::new(SchedulerConfig::default()).unwrap();
    let local = std::env::current_exe().unwrap();
    for source in [
        "magnet:?xt=urn:btih:abc&tr=https://user:SOURCE_SECRET@tracker.invalid".to_owned(),
        local.to_str().unwrap().to_owned(),
        "ftp://user:SOURCE_SECRET@host.invalid/file".to_owned(),
        "SOURCE_SECRET://host.invalid/file".to_owned(),
    ] {
        let request = RpcRequest::new(
            1,
            "task.create",
            Some(json!({"id":"rejected","source":source,"destination":"output"})),
        );
        let response = RpcDispatcher::dispatch(&mut core, &request).unwrap();
        assert_eq!(response.error.as_ref().unwrap().code, -32602);
        assert!(
            !serde_json::to_string(&response)
                .unwrap()
                .contains("SOURCE_SECRET")
        );
        assert_eq!(core.tasks.list().count(), 0);
        assert!(core.repository.list().unwrap().is_empty());
        assert!(core.tasks.drain_events().is_empty());
    }
}

#[test]
fn legacy_non_http_notification_cannot_create_a_task() {
    let mut core = Core::new(SchedulerConfig::default()).unwrap();
    let request = RpcRequest::notification(
        "task.create",
        Some(
            json!({"id":"notification","source":"magnet:?xt=urn:btih:abc","destination":"output"}),
        ),
    );
    assert!(RpcDispatcher::dispatch(&mut core, &request).is_none());
    assert!(core.repository.list().unwrap().is_empty());
    assert_eq!(core.tasks.list().count(), 0);
    assert!(core.tasks.drain_events().is_empty());
}

#[test]
fn legacy_http_creation_retains_resolver_case_and_whitespace_behavior() {
    let mut core = Core::new(SchedulerConfig::default()).unwrap();
    for (index, source) in [
        "http://example.invalid/file",
        "https://example.invalid/file",
        "HTTP://example.invalid/file",
        "  HtTpS://example.invalid/file  ",
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("http-{index}");
        let response = RpcDispatcher::dispatch(
            &mut core,
            &RpcRequest::new(
                1,
                "task.create",
                Some(json!({"id":id,"source":source,"destination":"output"})),
            ),
        )
        .unwrap();
        assert!(response.error.is_none(), "{response:?}");
        assert_eq!(response.result.as_ref().unwrap()["source"], source);
        assert_eq!(
            core.repository
                .get(&TaskId::from(id))
                .unwrap()
                .unwrap()
                .source
                .as_str(),
            source
        );
    }
}

#[test]
fn legacy_get_and_list_hide_source_and_error_without_rewriting_storage() {
    let mut core = Core::new(SchedulerConfig::default()).unwrap();
    let sources = [
        "magnet:?xt=urn:btih:abc&tr=https://user:SOURCE_SECRET@tracker.invalid",
        "/private/SOURCE_SECRET/file.torrent",
        "ftp://user:SOURCE_SECRET@host.invalid/file",
    ];
    let mut stored = Vec::new();
    for (index, source) in sources.into_iter().enumerate() {
        let mut task = DownloadTask::new(
            format!("legacy-{index}"),
            DownloadSource::new(source),
            Destination::new("output"),
        );
        task.state = TaskState::Failed;
        task.progress = nexum_domain::Progress::new(123, Some(456));
        task.last_error = Some(format!("failed at {source}: ERROR_SECRET"));
        let record = StoredTask::from(task.clone());
        core.repository.insert(record.clone()).unwrap();
        core.tasks.restore(task).unwrap();
        stored.push(record);
    }
    for record in &stored {
        let response = RpcDispatcher::dispatch(
            &mut core,
            &RpcRequest::new(1, "task.get", Some(json!({"id":record.id.as_str()}))),
        )
        .unwrap();
        let view = response.result.unwrap();
        let encoded = view.to_string();
        assert!(!encoded.contains("SOURCE_SECRET"));
        assert!(!encoded.contains("ERROR_SECRET"));
        assert_eq!(view["state"], "Failed");
        assert_eq!(view["downloaded_bytes"], 123);
        assert_eq!(view["total_bytes"], 456);
        assert!(view["error"].as_str().unwrap().contains("reimport"));
        assert_eq!(
            core.repository.get(&record.id).unwrap().as_ref(),
            Some(record)
        );
    }
    let response =
        RpcDispatcher::dispatch(&mut core, &RpcRequest::new(1, "task.list", None)).unwrap();
    let encoded = serde_json::to_string(&response).unwrap();
    assert!(!encoded.contains("SOURCE_SECRET"));
    assert!(!encoded.contains("ERROR_SECRET"));
    assert_eq!(
        response.result.unwrap().as_array().unwrap().len(),
        stored.len()
    );
}

#[test]
fn http_views_keep_the_existing_source_error_and_progress() {
    let mut task = DownloadTask::new(
        "http-view",
        DownloadSource::new("https://example.invalid/file?version=2"),
        Destination::new("output"),
    );
    task.last_error = Some("connection interrupted".to_owned());
    task.progress = nexum_domain::Progress::new(123, Some(456));
    let view = TaskView::from(&task);
    assert_eq!(view.source, task.source.as_str());
    assert_eq!(view.error, task.last_error);
    assert_eq!(view.downloaded_bytes, 123);
    assert_eq!(view.total_bytes, Some(456));
}
