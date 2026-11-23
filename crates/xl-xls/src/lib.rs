//! Read Excel 97-2003 workbooks (.xls): the BIFF8 records of the `Workbook` stream of a
//! compound file, into the same [`xl_cells::Workbook`] an .xlsx file gives.
//!
//! What is read: worksheets with their cells (values, formulas, shared and array
//! formulas), cell formats (number format, font, fill, borders, alignment), column and
//! row attributes, merged ranges, frozen panes, sheet visibility, the 1904 date system
//! and defined names. Not read: chart sheets, macro sheets, charts, pictures, comments,
//! hyperlinks, conditional formats, validations and formatted runs of text. Files of
//! Excel 95 and older (BIFF5 and below) and encrypted files are refused with an error.

mod biff;
mod cfb;
mod formula;
mod ftab;

pub use biff::{calc_settings, read_bytes, read_with_notes, Notes};
pub use cfb::is_cfb;

#[derive(Debug)]
pub enum Error {
    /// The file is damaged or not what it claims.
    Invalid(String),
    /// A valid file this crate does not read (older format, encryption).
    Unsupported(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Invalid(m) => write!(f, "invalid .xls file: {m}"),
            Error::Unsupported(m) => write!(f, "unsupported .xls file: {m}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
