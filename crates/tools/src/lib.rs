//! Shared plumbing for the tools in this directory.
//!
//! Only things more than one binary needs live here. A tool that is the sole
//! user of some logic keeps it in its own `src/bin/<name>.rs` or its own
//! `src/<name>/` module, so that the surface everyone depends on stays small.

pub mod collect_rows;
pub mod disasm;
pub mod funcs;
pub mod outputs;
pub mod paths;
pub mod pattern;
pub mod paz;
pub mod pe;
pub mod records;
pub mod scan;
pub mod sha256;
pub mod staticinfo;
