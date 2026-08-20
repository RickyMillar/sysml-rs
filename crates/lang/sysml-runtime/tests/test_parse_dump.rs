use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_parsed_elements() {
        let source = r#"
package Test {
    state def SM1 {
        entry; then s1;
        state s1 {
            entry action { boilerTemp = 20; machineReady = 0; }
        }
        state s2;
        transition t1 first s1 accept go then s2;
    }
}
"#;
        let parser = TreeSitterParser::new();
        let files = vec![SysmlFile::new("test.sysml", source)];
        let result = parser.parse(&files);

        println!("=== Parse diagnostics ({}) ===", result.diagnostics.len());
        for d in &result.diagnostics {
            println!("  [{:?}] {}", d.severity, d.message);
        }

        println!("\n=== ALL elements ({}) ===", result.graph.element_count());
        for e in result.graph.elements.values() {
            let parent_name = e
                .owner
                .as_ref()
                .and_then(|oid| result.graph.get_element(oid))
                .and_then(|p| p.name.as_deref())
                .unwrap_or("?");
            println!(
                "  {:?} name={:?} parent={} id={}",
                e.kind, e.name, parent_name, e.id
            );
            if !e.props.is_empty() {
                for (k, v) in e.props.iter() {
                    println!("    prop {}: {:?}", k, v);
                }
            }
        }

        // Check children of s1
        println!("\n=== Children of s1 ===");
        for e in result.graph.elements.values() {
            if e.name.as_deref() == Some("s1") {
                for child in result.graph.children_of(&e.id) {
                    println!("  child: {:?} name={:?}", child.kind, child.name);
                    for grandchild in result.graph.children_of(&child.id) {
                        println!(
                            "    grandchild: {:?} name={:?}",
                            grandchild.kind, grandchild.name
                        );
                        for (k, v) in grandchild.props.iter() {
                            println!("      prop {}: {:?}", k, v);
                        }
                    }
                }
            }
        }
    }
}
