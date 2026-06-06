//! Stream commits from a Repo into `DuckDB` via the
//! N gix workers → bounded crossbeam channel → 1 Appender thread pattern.
//! See spec §3.2.2.
//!
//! Threading model: `duckdb::Connection` contains `RefCell`, which is `!Sync`.
//! Therefore `&FactsDb` is `!Send` and the Appender must run on the thread that
//! owns the `FactsDb`. We flip the design: the **producer** runs in a scoped
//! spawned thread (Repo is `Send + Sync`) and the **consumer** (Appender) runs
//! on the calling thread.

use crossbeam_channel::bounded;
use duckdb::Appender;

use super::FactsDb;
use crate::repo::Repo;
use crate::{BcaError, ChangeType, CommitEvent, Options, Result};

const CHANNEL_CAPACITY: usize = 64;

#[derive(Debug, Default)]
pub struct IngestStats {
    pub commits_ingested: usize,
    pub changes_ingested: usize,
}

impl FactsDb {
    /// # Panics
    ///
    /// Panics if the producer thread panics (internal logic error, not expected in normal use).
    pub fn ingest<R: Repo>(&self, repo: &R, opts: &Options) -> Result<IngestStats> {
        // Plan 1: single producer gix walker → bounded channel → Appender on calling thread.
        // Plan 4 will fan out N producers.
        let (tx, rx) = bounded::<CommitEvent>(CHANNEL_CAPACITY);

        std::thread::scope(|s| -> Result<IngestStats> {
            // Producer: runs in a scoped thread. Repo: Send + Sync, opts borrows fine.
            let producer = s.spawn(|| -> Result<()> {
                let walk = repo.walk_commits(opts)?;
                for event in walk {
                    let event = event?;
                    tx.send(event)
                        .map_err(|e| BcaError::Analysis(format!("channel send: {e}")))?;
                }
                drop(tx); // Signals consumer to stop.
                Ok(())
            });

            // Consumer: runs on the calling thread — FactsDb / Connection stays single-threaded.
            let stats = ingest_loop(self, rx)?;

            producer.join().expect("producer panicked")?;
            Ok(stats)
        })
    }
}

fn ingest_loop(db: &FactsDb, rx: crossbeam_channel::Receiver<CommitEvent>) -> Result<IngestStats> {
    let mut stats = IngestStats::default();
    let mut commits_app = db
        .conn()
        .appender("commits")
        .map_err(|e| BcaError::Analysis(format!("appender commits: {e}")))?;
    let mut changes_app = db
        .conn()
        .appender("changes")
        .map_err(|e| BcaError::Analysis(format!("appender changes: {e}")))?;

    for event in rx {
        append_commit(&mut commits_app, &event)?;
        for ch in &event.changes {
            append_change(&mut changes_app, &event.rev, ch)?;
            stats.changes_ingested += 1;
        }
        stats.commits_ingested += 1;
    }
    commits_app
        .flush()
        .map_err(|e| BcaError::Analysis(format!("flush commits: {e}")))?;
    changes_app
        .flush()
        .map_err(|e| BcaError::Analysis(format!("flush changes: {e}")))?;
    Ok(stats)
}

/// Format a `time::Date` as `YYYY-MM-DD` without the `formatting` feature.
fn format_date(date: time::Date) -> String {
    let y = date.year();
    let m = date.month() as u8;
    let d = date.day();
    format!("{y:04}-{m:02}-{d:02}")
}

fn append_commit(app: &mut Appender<'_>, e: &CommitEvent) -> Result<()> {
    use duckdb::params;
    let date_str = format_date(e.date);
    app.append_row(params![
        e.rev,
        e.author_email,
        e.author_name,
        e.committer_email,
        e.author_email,         // canonical_author — Plan 4 fills properly
        Option::<String>::None, // ai_attribution
        date_str,
        e.message,
        e.parents.len() > 1,
        i32::try_from(e.parents.len()).unwrap_or(i32::MAX),
        // Kamei nulls — Plan 4 fills
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<bool>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<i32>::None,
    ])
    .map_err(|err| BcaError::Analysis(format!("append commit: {err}")))?;
    Ok(())
}

fn append_change(app: &mut Appender<'_>, rev: &str, ch: &crate::FileChange) -> Result<()> {
    use duckdb::params;
    let (type_str, rename_from, similarity) = match &ch.change_type {
        ChangeType::Added => ("added", None, None),
        ChangeType::Modified => ("modified", None, None),
        ChangeType::Deleted => ("deleted", None, None),
        ChangeType::Renamed { from, similarity } => {
            ("renamed", Some(from.as_str()), Some(i32::from(*similarity)))
        }
        ChangeType::Copied { from, similarity } => {
            ("copied", Some(from.as_str()), Some(i32::from(*similarity)))
        }
        ChangeType::BinaryOrUnknown => ("binary", None, None),
    };
    app.append_row(params![
        rev,
        ch.path,
        type_str,
        rename_from,
        similarity,
        i32::try_from(ch.loc_added).unwrap_or(i32::MAX),
        i32::try_from(ch.loc_deleted).unwrap_or(i32::MAX),
    ])
    .map_err(|err| BcaError::Analysis(format!("append change: {err}")))?;
    Ok(())
}
