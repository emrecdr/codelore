//! `revisions` analysis — file → revision count.
//! Code-maat parity output: (entity, n-revs).
//! See spec §1.1 ("authors and revisions are addressable standalone").

use crate::facts::FactsDb;
use crate::{BcaError, Options, Result};

pub fn run_revisions(db: &FactsDb, opts: &Options) -> Result<Vec<(String, u32)>> {
    let limit = opts
        .rows_limit
        .map(|n| format!(" LIMIT {n}"))
        .unwrap_or_default();
    let sql = format!(
        "SELECT path, COUNT(DISTINCT rev) AS n_revs
         FROM changes
         GROUP BY path
         HAVING n_revs >= {min}
         ORDER BY n_revs DESC, path ASC{limit}",
        min = opts.min_revs,
        limit = limit,
    );
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| BcaError::Analysis(format!("prepare revisions: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
            ))
        })
        .map_err(|e| BcaError::Analysis(format!("query revisions: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| BcaError::Analysis(format!("collect revisions: {e}")))
}
