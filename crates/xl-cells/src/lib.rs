//! Read and write the cells of Excel workbooks (xlsx, xlsm) as one columnar table.
//!
//! The table ([`Cells`]) has one row per cell and plain vectors as columns, which is
//! the layout of a dataframe. Nothing here depends on a dataframe library.

mod model;
mod read;
mod update;
mod write;
mod xml;

pub use model::*;
pub use read::{read, read_bytes, rich_text};
pub use update::{update_bytes, update_bytes_report};
pub use write::{part_xml, without_rel_ids, write, write_bytes, MAX_TEXT};
pub use xml::unescape_xstring;

#[derive(Debug)]
pub enum Error {
    Opc(xl_opc::Error),
    Xml(String),
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Opc(e) => write!(f, "{e}"),
            Error::Xml(m) => write!(f, "XML: {m}"),
            Error::Invalid(m) => write!(f, "invalid workbook: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<xl_opc::Error> for Error {
    fn from(e: xl_opc::Error) -> Self {
        Error::Opc(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
