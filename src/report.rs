//! Dashboard data + HTML. Ported from `khessman/ica-sync`'s `report.rs`,
//! but the source of rows changed: the old version re-parsed and
//! re-categorized every receipt JSON file on every report build; this one
//! reads already-categorized `Receipt`s straight out of `FsStore` — `sync()`
//! (kvitto-core) does the parse+categorize step once, at sync time, and
//! caches it. `report_template.html` is reused verbatim — its
//! `ItemRow{name,category,key,amount,month,r}` JSON shape barely changed
//! (`key` is new: the chain-prefixed override key, so the client can let
//! two chains' identically-named products be recategorized independently).

use crate::sync_meta;
use kvitto_core::{categorize::override_key, FsStore, ProfileId, SourceId};
use serde::Serialize;

#[derive(Serialize)]
struct ItemRow {
    name: String,
    category: String,
    key: String,
    amount: f64,
    month: String,
    #[serde(rename = "r")]
    receipt: usize,
}

#[derive(Serialize)]
struct SyncInfo {
    profile: String,
    source: String,
    source_label: String,
    implemented: bool,
    unix_seconds: Option<u64>,
    newly_fetched: Option<usize>,
    total_receipts: Option<usize>,
}

#[derive(Serialize)]
struct ReportData {
    items: Vec<ItemRow>,
    receipts_count: usize,
    syncs: Vec<SyncInfo>,
}

/// Sources shown as sync buttons, whether or not they're wired up yet —
/// Hemköp shows disabled ("kommer snart") until it has a `ReceiptSource`.
const SOURCE_ROWS: &[(SourceId, &str, bool)] =
    &[(kvitto_core::WILLYS, "Willys", true), (kvitto_core::ICA, "ICA", true)];
// Hemköp added here once it exists: (HEMKOP, "Hemköp", true).

fn build_report_data(store: &FsStore, profiles: &[ProfileId], data_root: &str) -> anyhow::Result<ReportData> {
    let receipts = store.merged(profiles)?;

    let mut items: Vec<ItemRow> = Vec::new();
    for (i, r) in receipts.iter().enumerate() {
        let chain = r.id.source.0;
        let month = r.date().format("%Y-%m").to_string();
        for line in &r.lines {
            items.push(ItemRow {
                name: line.description.clone(),
                category: line.category.clone().unwrap_or_else(|| "Okategoriserat".to_string()),
                key: override_key(chain, line),
                amount: line.amount.kr(),
                month: month.clone(),
                receipt: i,
            });
        }
    }

    let mut syncs = Vec::new();
    for profile in profiles {
        for (source, label, implemented) in SOURCE_ROWS {
            let last = sync_meta::read(data_root, profile, *source);
            syncs.push(SyncInfo {
                profile: profile.0.clone(),
                source: source.0.to_string(),
                source_label: label.to_string(),
                implemented: *implemented,
                unix_seconds: last.as_ref().map(|l| l.unix_seconds),
                newly_fetched: last.as_ref().map(|l| l.newly_fetched),
                total_receipts: last.as_ref().map(|l| l.total_receipts),
            });
        }
    }

    Ok(ReportData { items, receipts_count: receipts.len(), syncs })
}

const TEMPLATE: &str = include_str!("report_template.html");

pub fn render_html(store: &FsStore, profiles: &[ProfileId], data_root: &str) -> anyhow::Result<String> {
    let data = build_report_data(store, profiles, data_root)?;
    let json = serde_json::to_string(&data)?;
    Ok(TEMPLATE.replace("__REPORT_DATA__", &json))
}
