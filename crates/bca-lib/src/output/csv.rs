//! CSV emitters. Headers match code-maat exactly for golden-test parity.

use std::io::Write;

use crate::{BcaError, Result};

pub fn write_revisions_csv<W: Write>(rows: &[(String, u32)], w: &mut W) -> Result<()> {
    writeln!(w, "entity,n-revs").map_err(BcaError::Io)?;
    for (entity, n) in rows {
        if entity.contains(',') || entity.contains('"') || entity.contains('\n') {
            let escaped = entity.replace('"', "\"\"");
            writeln!(w, "\"{escaped}\",{n}").map_err(BcaError::Io)?;
        } else {
            writeln!(w, "{entity},{n}").map_err(BcaError::Io)?;
        }
    }
    Ok(())
}
