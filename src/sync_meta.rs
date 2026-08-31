//! Small per-`(profile, source)` marker (`data/{profile}/last_sync_{source}.json`)
//! saying only when a sync last ran and what it found — never receipt
//! content. Ported from `khessman/ica-sync`'s `sync_meta.rs`, genericized
//! off the ICA-specific field name and keyed per source now that a profile
//! can have more than one chain.

use kvitto_core::{ProfileId, SourceId, SyncReport};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct LastSync {
    pub unix_seconds: u64,
    pub newly_fetched: usize,
    pub total_receipts: usize,
}

fn path(data_root: &str, profile: &ProfileId, source: SourceId) -> std::path::PathBuf {
    std::path::Path::new(data_root).join(profile.dir()).join(format!("last_sync_{}.json", source.0))
}

pub fn write(data_root: &str, profile: &ProfileId, source: SourceId, report: &SyncReport) -> anyhow::Result<()> {
    let unix_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let payload = LastSync {
        unix_seconds,
        newly_fetched: report.downloaded,
        total_receipts: report.listed,
    };
    let p = path(data_root, profile, source);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, serde_json::to_string(&payload)?)?;
    Ok(())
}

pub fn read(data_root: &str, profile: &ProfileId, source: SourceId) -> Option<LastSync> {
    let raw = std::fs::read_to_string(path(data_root, profile, source)).ok()?;
    serde_json::from_str(&raw).ok()
}
