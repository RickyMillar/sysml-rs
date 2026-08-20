//! `sysml update` — force lock refresh.

use crate::common::CliError;

pub fn run(quiet: bool, json_output: bool) -> Result<(), CliError> {
    crate::lock::run_with_options(true, quiet, json_output)
}
