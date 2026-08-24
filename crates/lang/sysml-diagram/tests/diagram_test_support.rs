use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

pub fn parse_sysml(source: &str) -> sysml_core::ModelGraph {
    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new("test.sysml", source)];
    let result = parser.parse(&files);
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == sysml_span::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    result.graph
}
