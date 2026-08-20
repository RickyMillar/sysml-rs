use sysml_service::SysmlService;

use crate::common::CliError;

/// Run `sysml eval "expr"`.
pub fn run(expr: &str) -> Result<(), CliError> {
    let service = SysmlService::empty();

    let value = service
        .eval_expression(expr, &[])
        .map_err(|e| CliError::user(format!("{e}")))?;
    println!("{value}");

    Ok(())
}
