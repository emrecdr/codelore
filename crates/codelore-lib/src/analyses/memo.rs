//! Process-local analysis memos, owned by the analyses layer.
//!
//! These caches are keyed on analyses row/graph types, so they must not live
//! on [`crate::facts::FactsDb`] — that would make the `facts` layer depend on
//! `analyses` types, a genuine module cycle. Instead `FactsDb` holds one
//! type-erased slot map (`analysis_memo::<T>()`); each memo below is a `T`
//! that the analyses layer stores and retrieves through it, keyed per-FactsDb
//! and shared for that connection's lifetime.
//!
//! Every memo stores the FULL, un-row-limited result: callers re-apply their
//! own `rows_limit` after the lookup so a `--rows N` choice never poisons the
//! shared entry. `RefCell` (not a `Mutex`) because the `DuckDB` `Connection`
//! is `!Send + !Sync` and every analysis runs on the single connection-owning
//! thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::clones::ClonesRow;
use super::coupling::{CouplingMemoKey, CouplingRow};
use super::import_graph::ImportGraph;

/// Memo for [`crate::analyses::coupling::run_coupling`], which is pure per
/// `(db, coupling-affecting opts)` yet is invoked 2-5× per CLI run on
/// identical inputs (code-health, centrality, communities, clone-coupling,
/// and the SPA dashboard all re-derive the same global coupling graph). The
/// stored value is the full, Fisher-filtered, un-row-limited `Vec`.
#[derive(Default)]
pub(crate) struct CouplingMemo(RefCell<HashMap<CouplingMemoKey, Rc<Vec<CouplingRow>>>>);

impl CouplingMemo {
    /// Look up a memoised coupling result for `key`. `None` on a miss; the
    /// caller then computes and stores.
    pub(crate) fn get(&self, key: &CouplingMemoKey) -> Option<Rc<Vec<CouplingRow>>> {
        self.0.borrow().get(key).cloned()
    }

    /// Store the full, un-row-limited coupling result under `key`.
    pub(crate) fn put(&self, key: CouplingMemoKey, rows: Rc<Vec<CouplingRow>>) {
        self.0.borrow_mut().insert(key, rows);
    }
}

/// Single-slot memo for the structural import graph
/// ([`crate::analyses::import_graph::build_import_graph`]). The graph is a
/// pure function of the immutable `imports` table, yet a `--format spa` render
/// or a `codelore check` arch-suite rebuilds it (SQL scan + path interning +
/// adjacency) once per arch analysis; a shared `Rc` collapses those into one
/// build.
#[derive(Default)]
pub(crate) struct ImportGraphMemo(RefCell<Option<Rc<ImportGraph>>>);

impl ImportGraphMemo {
    /// Shared handle to the memoised import graph, if built this run.
    pub(crate) fn get(&self) -> Option<Rc<ImportGraph>> {
        self.0.borrow().clone()
    }

    /// Store the import graph for reuse across arch analyses.
    pub(crate) fn put(&self, graph: Rc<ImportGraph>) {
        *self.0.borrow_mut() = Some(graph);
    }
}

/// Single-slot memo for [`crate::analyses::clones::run_clones_memoised`].
/// `run_clones` walks the working tree and tree-sitter-fingerprints every
/// Tier-1 function — an O(files) filesystem + parse pass with no `changes` /
/// `imports` dependency, so its result is fixed for a given (repo,
/// clone-affecting opts) pair. The agent-loop gate's projected-health engine
/// runs code-health twice on one `FactsDb` (HEAD baseline vs the
/// substituted-complexity projection); both scoped runs re-walk clones over
/// the SAME working tree, so the second walk is pure waste.
#[derive(Default)]
pub(crate) struct ClonesMemo(RefCell<Option<Rc<Vec<ClonesRow>>>>);

impl ClonesMemo {
    /// Shared handle to the memoised clones walk, if computed this run.
    pub(crate) fn get(&self) -> Option<Rc<Vec<ClonesRow>>> {
        self.0.borrow().clone()
    }

    /// Store the clones walk for reuse across the two agent-loop scoped scans.
    pub(crate) fn put(&self, rows: Rc<Vec<ClonesRow>>) {
        *self.0.borrow_mut() = Some(rows);
    }
}
