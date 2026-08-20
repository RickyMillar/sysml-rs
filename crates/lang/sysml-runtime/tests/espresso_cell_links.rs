//! espresso-production-cell — exchange-plane link classification (Stage B).
//!
//! CELL-LINK-01 / EX-POWER / EX-SIGNAL / EX-MSG: the workspace classifies its
//! declared connectors into PowerBond / SignalLink / MessageChannel with the
//! expected distribution and no unexplained Unknown links.

mod common;
use common::load_example_graph;
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::flows::compile_ports;
use sysml_runtime::links::classify_links;

const FIXTURE: &str = "espresso-production-cell";

#[test]
fn cell_links_classify_into_three_planes() {
    let compiler = ModelCompiler::new(load_example_graph(FIXTURE));
    let registry = compile_ports(compiler.graph());
    let (link_graph, diags) = classify_links(compiler.graph(), &registry);
    let dist = link_graph.class_distribution();

    eprintln!(
        "espresso-cell link distribution: power_bond={} signal_link={} \
         message_channel={} unknown={} (total={})",
        dist.power_bond, dist.signal_link, dist.message_channel, dist.unknown,
        link_graph.len()
    );
    for d in &diags {
        eprintln!("  classify diag: {d}");
    }

    // Exact per-class distribution — pinned so a per-class regression cannot
    // slip under a lower bound. Counts (each flow + its declared connector is
    // interned separately, per the corpus convention):
    //   PowerBond      = 14  (station hydraulic supply/return + thermal, ×2/pair,
    //                         plus the LinkCorpus power pair)
    //   SignalLink     =  4  (LinkCorpus pressure reading + the cross-dimension
    //                         negative, ×2/pair)
    //   MessageChannel =  4  (LinkCorpus dispatcher→group + supervisor→granted)
    //   Unknown        =  0  (the negative reconciles structurally; no unexplained links)
    assert_eq!(dist.power_bond, 14, "PowerBond count");
    assert_eq!(dist.signal_link, 4, "SignalLink count");
    assert_eq!(dist.message_channel, 4, "MessageChannel count");
    assert_eq!(dist.unknown, 0, "no unexplained Unknown links");
    assert_eq!(
        link_graph.len(),
        dist.power_bond + dist.signal_link + dist.message_channel + dist.unknown,
        "every link accounted for in exactly one class"
    );
}
