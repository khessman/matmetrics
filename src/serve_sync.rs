//! The Uppdatera buttons — one per (profile, source), not one per profile.
//! Adapted from an early draft of this file written before `kvitto-ica`
//! existed; the only real change is that a job now targets a single source
//! instead of walking every source for a profile in one go, so clicking
//! "Willys" doesn't make you wait through an ICA BankID round too.
//!
//! Three endpoints:
//!   POST /api/sync/start                    {profile, source} -> job id
//!   GET  /api/sync/status/{profile}/{source} poll for progress
//!   POST /api/reparse                       re-parse + re-categorize, no login needed

use kvitto_core::{
    reprocess, sync, FsStore, Job, JobState, ProfileId, ReceiptSource, Result, SessionStore,
    SourceId,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub data_root: String,
    pub store: FsStore,
    pub profiles: Vec<ProfileId>,
    pub base_url: Option<String>,
    /// In memory for the lifetime of the process. Never written to disk.
    pub sessions: Arc<dyn SessionStore>,
    pub jobs: JobRegistry,
}

impl AppState {
    pub fn new(data_root: String, profiles: Vec<ProfileId>) -> Self {
        Self {
            store: FsStore::new(&data_root),
            data_root,
            profiles,
            base_url: None,
            sessions: Arc::new(kvitto_core::MemorySessionStore::new()),
            jobs: JobRegistry::default(),
        }
    }

    /// Adding a chain is one line here. Sources are built per job because
    /// each holds a live HTTP session; `SessionStore` is what's shared, kept
    /// per profile.
    fn sources(&self) -> Result<Vec<Box<dyn ReceiptSource>>> {
        let report_url = self.base_url.as_ref().map(|b| format!("{b}/report.html"));

        let mut willys = kvitto_willys::AxfoodSource::new(kvitto_willys::Chain::Willys, self.sessions.clone())?;
        willys.return_url = report_url.clone();

        let mut hemkop = kvitto_willys::AxfoodSource::new(kvitto_willys::Chain::Hemkop, self.sessions.clone())?;
        hemkop.return_url = report_url;

        Ok(vec![
            Box::new(kvitto_ica::Ica::new(self.sessions.clone())?),
            Box::new(willys),
            Box::new(hemkop),
        ])
    }
}

#[derive(Default)]
pub struct JobRegistry {
    inner: Mutex<HashMap<u64, Job>>,
    /// One active job per (profile, source) — lets two sources for the same
    /// profile sync concurrently, but not the same source twice.
    active: Mutex<HashMap<(ProfileId, SourceId), u64>>,
    next: Mutex<u64>,
}

impl JobRegistry {
    pub fn get(&self, id: u64) -> Option<JobState> {
        self.inner.lock().unwrap().get(&id).map(|j| j.snapshot())
    }

    /// The last job started for this (profile, source), regardless of
    /// whether it's still running — this is what status polling looks up.
    pub fn status_for(&self, p: &ProfileId, s: SourceId) -> Option<JobState> {
        let id = *self.active.lock().unwrap().get(&(p.clone(), s))?;
        self.get(id)
    }

    fn active_for(&self, p: &ProfileId, s: SourceId) -> Option<u64> {
        let id = *self.active.lock().unwrap().get(&(p.clone(), s))?;
        // Stale entries (job finished but never cleared, e.g. after a
        // crash) shouldn't block a new run forever.
        if self.get(id).is_some_and(|j| !matches!(j.phase, kvitto_core::Phase::Done | kvitto_core::Phase::Failed { .. })) {
            Some(id)
        } else {
            None
        }
    }

    fn create(&self, p: ProfileId, s: SourceId) -> (Job, u64) {
        let mut n = self.next.lock().unwrap();
        *n += 1;
        let id = *n;
        let job = Job::new(id, p.clone());
        self.inner.lock().unwrap().insert(id, job.clone());
        self.active.lock().unwrap().insert((p, s), id);
        (job, id)
    }
}

/// POST /api/sync/start — `source` selects exactly one chain, matched by
/// `SourceId`'s inner name (`"willys"`/`"ica"`).
pub fn start_sync(app: Arc<AppState>, profile: ProfileId, source: SourceId) -> Result<u64> {
    if let Some(existing) = app.jobs.active_for(&profile, source) {
        return Ok(existing);
    }
    let (job, id) = app.jobs.create(profile.clone(), source);

    // Wrapped so a panic anywhere in the sync (a `todo!()` in a source's
    // `list`/`fetch`, say) marks the job Failed instead of leaving it frozen
    // on whatever phase it last reached — a bare `tokio::spawn` swallows a
    // panicking task's result if nothing ever awaits its `JoinHandle`, which
    // is exactly what happened before this existed.
    let job_for_panic = job.clone();
    tokio::spawn(async move {
        if let Err(e) = tokio::spawn(run_sync(app, profile, source, job)).await {
            if e.is_panic() {
                job_for_panic.set(kvitto_core::Phase::Failed {
                    error: format!("internal error (panic): {e}"),
                });
            }
        }
    });

    Ok(id)
}

async fn run_sync(app: Arc<AppState>, profile: ProfileId, source: SourceId, job: Job) {
    {
        let mut srcs = match app.sources() {
            Ok(s) => s,
            Err(e) => return job.set(kvitto_core::Phase::Failed { error: e.to_string() }),
        };
        let Some(src) = srcs.iter_mut().find(|s| s.id() == source) else {
            return job.set(kvitto_core::Phase::Failed {
                error: format!("unknown source {}", source.0),
            });
        };

        let name = src.id().to_string();
        let ui = job.ui_for(&name);
        // Loaded fresh, not cached: has to see overrides written by the
        // dashboard's recategorize UI moments ago, not whatever was on disk
        // at server startup.
        let cat = match crate::load_categorizer() {
            Ok(c) => c,
            Err(e) => return job.set(kvitto_core::Phase::Failed { error: e.to_string() }),
        };
        let result =
            sync(src.as_mut(), &profile, &app.store, &app.store, &cat, None, &ui, Some(&job)).await;

        match result {
            Ok(rep) => {
                job.push_report(&name, &rep);
                let _ = crate::sync_meta::write(&app.data_root, &profile, source, &rep);
            }
            Err(e) => {
                tracing::warn!("{name}/{profile} failed: {e}");
                job.set(kvitto_core::Phase::Failed { error: e.to_string() });
                return;
            }
        }

        job.set(kvitto_core::Phase::Rebuilding);
        if let Err(e) = rebuild_report(&app) {
            tracing::warn!("report rebuild failed: {e}");
        }
        job.set(kvitto_core::Phase::Done);
    }
}

pub fn poll(app: &AppState, id: u64) -> Option<JobState> {
    app.jobs.get(id)
}

fn rebuild_report(app: &AppState) -> anyhow::Result<()> {
    let html = crate::report::render_html(&app.store, &app.profiles, &app.data_root)?;
    std::fs::create_dir_all("out")?;
    std::fs::write("out/report.html", html)?;
    Ok(())
}

/// POST /api/reparse — needs no session, safe to run any time. What runs
/// after editing `categories.toml` or landing a parser fix.
pub fn reparse(app: &AppState, force: bool) -> Result<()> {
    let cat = crate::load_categorizer().map_err(|e| kvitto_core::Error::Config(e.to_string()))?;
    for p in &app.profiles {
        for s in app.sources()? {
            let rep = reprocess(s.as_ref(), p, &app.store, &app.store, &cat, force)?;
            tracing::info!("{p}/{}: {rep:?}", s.id());
        }
    }
    rebuild_report(app).map_err(|e| kvitto_core::Error::Config(e.to_string()))
}
