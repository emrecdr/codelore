//! JSON output emitter for all 11 analyses.
//!
//! Each writer takes a slice of typed rows + writer; serializes as a JSON array.

use crate::analyses::{
    authors::AuthorsRow,
    churn::{AbsChurnRow, AuthorChurnRow, EntityChurnRow},
    clone_coupling::CloneCouplingRow,
    clones::ClonesRow,
    code_age::CodeAgeRow,
    code_health::CodeHealthRow,
    communication::CommunicationRow,
    coupling::CouplingRow,
    hotspots::HotspotRow,
    ownership::OwnershipRow,
    summary::SummaryRow,
};
use crate::{CodeLoreError, Result};
use std::io::Write;

fn write_json<W: Write, T: serde::Serialize>(rows: &[T], w: &mut W) -> Result<()> {
    serde_json::to_writer_pretty(w, rows).map_err(|e| CodeLoreError::Output(format!("json: {e}")))
}

pub fn write_revisions_json<W: Write>(rows: &[(String, u32)], w: &mut W) -> Result<()> {
    #[derive(serde::Serialize)]
    struct R<'a> {
        entity: &'a str,
        n_revs: u32,
    }
    let typed: Vec<R> = rows
        .iter()
        .map(|(p, n)| R {
            entity: p,
            n_revs: *n,
        })
        .collect();
    write_json(&typed, w)
}

pub fn write_hotspots_json<W: Write>(rows: &[HotspotRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_code_health_json<W: Write>(rows: &[CodeHealthRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_code_age_json<W: Write>(rows: &[CodeAgeRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_abs_churn_json<W: Write>(rows: &[AbsChurnRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_author_churn_json<W: Write>(rows: &[AuthorChurnRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_entity_churn_json<W: Write>(rows: &[EntityChurnRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_communication_json<W: Write>(rows: &[CommunicationRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_ownership_json<W: Write>(rows: &[OwnershipRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_coupling_json<W: Write>(rows: &[CouplingRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_summary_json<W: Write>(rows: &[SummaryRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_clones_json<W: Write>(rows: &[ClonesRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_authors_json<W: Write>(rows: &[AuthorsRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}

pub fn write_soc_json<W: Write>(
    rows: &[crate::analyses::soc::SocRow],
    w: &mut W,
) -> Result<()> {
    write_json(rows, w)
}

pub fn write_messages_json<W: Write>(
    rows: &[crate::analyses::messages::MessagesRow],
    w: &mut W,
) -> Result<()> {
    write_json(rows, w)
}

pub fn write_main_dev_json<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
) -> Result<()> {
    write_json(rows, w)
}

pub fn write_entity_effort_json<W: Write>(
    rows: &[crate::analyses::entity_effort::EntityEffortRow],
    w: &mut W,
) -> Result<()> {
    write_json(rows, w)
}

pub fn write_entity_ownership_json<W: Write>(
    rows: &[crate::analyses::entity_ownership::EntityOwnershipRow],
    w: &mut W,
) -> Result<()> {
    write_json(rows, w)
}

pub fn write_clone_coupling_json<W: Write>(rows: &[CloneCouplingRow], w: &mut W) -> Result<()> {
    write_json(rows, w)
}
