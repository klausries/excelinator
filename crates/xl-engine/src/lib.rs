//! The logic of moving Excel workbooks into a database application, free of any
//! dataframe library.
//!
//! * [`lift`]: the formula engine. Formulas filled down a column become one column
//!   expression ([`ir`]), checked against the values Excel stored; [`lift::calculate_with`]
//!   works out a workbook's formulas. The expressions are run by an [`ir::Executor`]:
//!   Polars in Rust (`xl-polars`), Polars in Python, ...
//! * [`data`]: typed tables ([`data::Frame`]), the form in which sheets and regions leave
//!   and data comes back; [`frame`]: parts of a sheet as frames.
//! * [`regions`], [`apply`], [`codegen`], [`store`]: what a workbook holds, data written
//!   back into it, the workbook as a Polars program, and a store of its data ([`long`]:
//!   single cells as rows, their form in a store).

pub mod apply;
pub mod codegen;
pub mod data;
pub mod frame;
pub mod ir;
pub mod lift;
pub mod long;
pub mod plans;
pub mod port;
pub mod pycode;
pub mod regions;
pub mod rows;
pub mod stack;
pub mod store;
pub mod view;
