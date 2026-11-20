use nexum_core::Core;
use nexum_domain::{Destination, DownloadSource};
use nexum_resolver::{ResolveKind, magnet::parse_magnet};
use nexum_scheduler::SchedulerConfig;
use nexum_storage::{SqliteRepository, TaskRepository, origin::random_id};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("nexum-magnet-core-{}", random_id().unwrap()));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn core(&self) -> Core<SqliteRepository> {
        Core::with_repository(
            SchedulerConfig::default(),
            SqliteRepository::open(self.0.join("nexum.sqlite")).unwrap(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn core_resolution_uses_strict_identity_validation_without_creating_a_task() {
    let fixture = Fixture::new();
    let mut core = fixture.core();
    for source in [
        "MAGNET:?xt=urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH",
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&xt=urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH&tr=https://tracker.invalid/PASSKEY_SENTINEL",
    ] {
        let resolved = core.resolve_source(source).unwrap();
        assert_eq!(resolved.kind, ResolveKind::Magnet);
        assert_eq!(resolved.source, source);
        assert_eq!(
            parse_magnet(&resolved.source).unwrap().info_hash_hex(),
            "0123456789abcdef0123456789abcdef01234567"
        );
        assert!(!format!("{resolved:?}").contains("PASSKEY_SENTINEL"));
    }
    assert!(core.repository.list().unwrap().is_empty());
    assert_eq!(core.tasks.list().count(), 0);
    assert!(core.drain_task_events().is_empty());
}

#[test]
fn invalid_magnet_creation_leaves_sqlite_memory_and_events_unchanged() {
    let fixture = Fixture::new();
    let mut core = fixture.core();
    let valid = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";
    let oversized = format!("{}{valid}", " ".repeat(8193 - valid.len()));
    let sources = [
        "magnet:?xt=urn:btih:abc".to_owned(),
        format!("{valid}&xt=urn:btih:000102030405060708090a0b0c0d0e0f10111213"),
        format!("{valid}&xt.1=urn:btih:0123456789abcdef0123456789abcdef01234567"),
        format!("{valid}&x=%FF"),
        format!("{valid}\n"),
        oversized,
        "magnet:?xt=urn:btih:PASSKEY_SENTINEL".into(),
        format!("\0{valid}&tr=https://tracker.invalid/PASSKEY_SENTINEL"),
        format!("ma\0gnet:{}", valid.split_once(':').unwrap().1),
    ];
    for (index, source) in sources.into_iter().enumerate() {
        assert!(core.resolve_source(&source).is_err());
        let error = core
            .create_task(
                format!("invalid-{index}"),
                DownloadSource::new(source),
                Destination::new("/unused/output"),
            )
            .unwrap_err();
        assert!(!format!("{error:?}").contains("PASSKEY_SENTINEL"));
        assert!(core.repository.list().unwrap().is_empty());
        assert_eq!(core.tasks.list().count(), 0);
        assert!(core.drain_task_events().is_empty());
    }
    drop(core);
    assert!(fixture.core().repository.list().unwrap().is_empty());
}
