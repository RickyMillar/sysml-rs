use sysml_span::{Diagnostic, HashMapSourceProvider, Span};

fn main() {
    let source = "package Demo {\n  part def Engine;\n  part engine : Engine;\n}\n";
    let span = Span::with_location("demo.sysml", 33, 39, 3, 8);
    let diag = Diagnostic::error("undefined type")
        .with_code("E010")
        .with_span(span)
        .with_note("declare 'Engine' or import it");

    let mut provider = HashMapSourceProvider::new();
    provider.insert("demo.sysml", source);

    let rendered = diag.render_snippet(&provider);
    println!("{rendered}");
}
