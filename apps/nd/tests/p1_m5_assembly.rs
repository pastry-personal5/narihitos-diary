#![forbid(unsafe_code)]

use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use diary_connectors::{AppleBridgeClient, AyumiCatalog, AyumiSource, SourceReadError};

static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn source_connector_assembles_without_reading_a_private_tree_or_starting_a_service() {
    let scratch = std::env::temp_dir().join(format!(
        "nd-p1-m5-assembly-{}-{}",
        std::process::id(),
        SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(scratch.join("icloud/Ayumi")).unwrap();
    assert!(matches!(
        AyumiCatalog::new(scratch.join("icloud"), "../Ayumi"),
        Err(SourceReadError::InvalidDirectoryName)
    ));
    let catalog = AyumiCatalog::new(scratch.join("icloud"), "Ayumi").unwrap();
    let bridge = AppleBridgeClient::new(
        scratch.join("missing-helper"),
        "Ayumi",
        scratch.join("handoff"),
        Duration::from_secs(1),
        1,
    )
    .unwrap();
    let _source = AyumiSource::new(catalog, bridge);
    let _ = fs::remove_dir_all(scratch);
}
