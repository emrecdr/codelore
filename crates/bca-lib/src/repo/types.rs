//! Public types for the Repo trait. Distinct from the
//! main types module to keep gix-coupled types isolated.

#[derive(Debug, Clone)]
pub struct CommitMetadata {
    pub rev: String,
    pub signed: bool,
    pub signed_by: Option<String>,
    pub signoffs: Vec<String>,
}
