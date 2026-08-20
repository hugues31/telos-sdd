use std::fmt::Write;

use telos_core::ids::IntentId;

use super::model::{IntentView, ViewSnapshot};

const RELATIONS: [&str; 9] = [
    "all",
    "refines",
    "requires",
    "excludes",
    "constrains",
    "verifies",
    "uses",
    "implements",
    "proves",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Dashboard,
    Graph,
    Intent(IntentId),
    Glossary,
    Coverage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkMode {
    Server,
    Export,
}

pub(crate) fn render(snapshot: &ViewSnapshot, page: Page, mode: LinkMode) -> Option<String> {
    let (title, body) = match page {
        Page::Dashboard => ("Dashboard".to_string(), dashboard(snapshot, page, mode)),
        Page::Graph => ("Relation graph".to_string(), graph(snapshot, page, mode)),
        Page::Intent(id) => {
            let intent = snapshot.intent(id)?;
            (
                format!("{} — {}", intent.id, intent.title),
                intent_page(snapshot, intent, page, mode),
            )
        }
        Page::Glossary => ("Glossary".to_string(), glossary(snapshot, page, mode)),
        Page::Coverage => ("Coverage".to_string(), coverage(snapshot)),
    };

    Some(layout(snapshot, page, mode, &title, &body))
}

fn layout(
    snapshot: &ViewSnapshot,
    current: Page,
    mode: LinkMode,
    title: &str,
    body: &str,
) -> String {
    let mut html = String::new();
    write!(
        html,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} · Telos</title><style>{}</style></head><body><header><a class=\"brand\" href=\"{}\">Telos</a><nav aria-label=\"Primary\">{}{}{}{}</nav><p class=\"state\">Project state: <strong>{}</strong></p></header><main>{}</main><footer>Generated from a validated Telos snapshot.</footer></body></html>",
        escape(title),
        CSS,
        escape(&href(current, PageTarget::Dashboard, mode)),
        nav_link("Dashboard", current, PageTarget::Dashboard, mode),
        nav_link("Graph", current, PageTarget::Graph, mode),
        nav_link("Glossary", current, PageTarget::Glossary, mode),
        nav_link("Coverage", current, PageTarget::Coverage, mode),
        escape(&snapshot.dashboard.state),
        body,
    )
    .expect("writing HTML to a String cannot fail");
    html
}

fn dashboard(snapshot: &ViewSnapshot, current: Page, mode: LinkMode) -> String {
    let coverage = &snapshot.coverage;
    let mut out = format!(
        "<section><p class=\"eyebrow\">Project overview</p><h1>Telos dashboard</h1><div class=\"metrics\"><article><strong>{}</strong><span>intents</span></article><article><strong>{}/{}</strong><span>implemented</span></article><article><strong>{}/{}</strong><span>scenarios proved</span></article><article><strong>{}</strong><span>notions</span></article></div></section>",
        coverage.intents_total,
        coverage.intents_implemented,
        coverage.intents_total,
        coverage.scenarios_proved,
        coverage.scenarios_total,
        coverage.notions,
    );

    out.push_str("<section><h2>Intents</h2><ol class=\"cards\">");
    for intent in &snapshot.intents {
        write!(
            out,
            "<li><a href=\"{}\"><span>{}</span><strong>{}</strong></a><small>{}</small></li>",
            escape(&intent_href(current, &intent.id, mode)),
            escape(&intent.id),
            escape(&intent.title),
            escape(&intent.status),
        )
        .unwrap();
    }
    out.push_str("</ol></section>");

    out.push_str("<section><h2>Working tree</h2>");
    if snapshot.dashboard.drift.is_empty() {
        out.push_str("<p>No unclaimed drift.</p>");
    } else {
        out.push_str("<ul>");
        for drift in &snapshot.dashboard.drift {
            write!(
                out,
                "<li><code>{}</code> — {}</li>",
                escape(&drift.path),
                escape(&drift.kind)
            )
            .unwrap();
        }
        out.push_str("</ul>");
    }
    if snapshot.dashboard.open_changes.is_empty() {
        out.push_str("<p>No open changes.</p>");
    } else {
        out.push_str("<ul>");
        for change in &snapshot.dashboard.open_changes {
            write!(
                out,
                "<li><strong>{}</strong> — {} ({} obligations)</li>",
                escape(&change.id),
                escape(&change.status),
                change.obligations.len()
            )
            .unwrap();
        }
        out.push_str("</ul>");
    }
    out.push_str("</section>");
    out
}

fn graph(snapshot: &ViewSnapshot, current: Page, mode: LinkMode) -> String {
    let mut out = String::from(
        "<section><p class=\"eyebrow\">Validated model</p><h1>Relation graph</h1><label for=\"relation-filter\">Relation</label><select id=\"relation-filter\">",
    );
    for relation in RELATIONS {
        write!(out, "<option value=\"{relation}\">{relation}</option>").unwrap();
    }
    out.push_str("</select></section><section><h2>Edges</h2><div class=\"table-wrap\"><table><thead><tr><th>From</th><th>Relation</th><th>To</th></tr></thead><tbody>");
    for (index, edge) in snapshot.edges.iter().enumerate() {
        let from = snapshot.nodes.iter().find(|node| node.id == edge.from);
        let to = snapshot.nodes.iter().find(|node| node.id == edge.to);
        write!(
            out,
            "<tr id=\"edge-{index}\" data-relation=\"{}\"><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
            escape(&edge.relation),
            graph_ref(snapshot, from, &edge.from, current, mode),
            escape(&edge.relation),
            graph_ref(snapshot, to, &edge.to, current, mode),
        )
        .unwrap();
    }
    out.push_str("</tbody></table></div></section><section><h2>Nodes</h2><ul class=\"nodes\">");
    for node in &snapshot.nodes {
        write!(
            out,
            "<li id=\"node-{}\"><span class=\"pill\">{}</span> {} <small>{}</small></li>",
            escape(&node.id),
            escape(&node.kind),
            graph_ref(snapshot, Some(node), &node.id, current, mode),
            escape(&node.label),
        )
        .unwrap();
    }
    out.push_str("</ul></section><script>const filter=document.getElementById('relation-filter');filter.addEventListener('change',()=>{for(const row of document.querySelectorAll('[data-relation]')){row.hidden=filter.value!=='all'&&row.dataset.relation!==filter.value;}});</script>");
    out
}

fn intent_page(
    snapshot: &ViewSnapshot,
    intent: &IntentView,
    current: Page,
    mode: LinkMode,
) -> String {
    let mut out = format!(
        "<article id=\"intent-{}\"><p class=\"eyebrow\">{} · {}</p><h1>{}</h1><p class=\"lede\">{}</p><details><summary>Canonical intent</summary><pre>{}</pre></details>",
        escape(&intent.id),
        escape(&intent.id),
        escape(&intent.status),
        escape(&intent.title),
        escape(&intent.telos),
        escape(&intent.canonical),
    );

    out.push_str("<section><h2>Relations</h2><ul>");
    let mut relation_count = 0;
    for edge in &snapshot.edges {
        let other = if edge.from == intent.id {
            Some((&edge.relation, &edge.to, "out"))
        } else if edge.to == intent.id {
            Some((&edge.relation, &edge.from, "in"))
        } else {
            None
        };
        let Some((relation, id, direction)) = other else {
            continue;
        };
        relation_count += 1;
        let node = snapshot.nodes.iter().find(|node| node.id == *id);
        write!(
            out,
            "<li><span class=\"pill\">{} {}</span> {}</li>",
            escape(direction),
            escape(relation),
            graph_ref(snapshot, node, id, current, mode),
        )
        .unwrap();
    }
    if relation_count == 0 {
        out.push_str("<li>No relations.</li>");
    }
    out.push_str("</ul></section><section><h2>Domain notions</h2><ul>");
    for notion in &intent.notions {
        let glossary = href(current, PageTarget::Glossary, mode);
        write!(
            out,
            "<li><a href=\"{}#notion-{}\">{}</a></li>",
            escape(&glossary),
            escape(notion),
            escape(notion),
        )
        .unwrap();
    }
    out.push_str("</ul></section><section><h2>Constraints</h2>");
    for constraint in &intent.constraints {
        write!(
            out,
            "<article id=\"constraint-{}\"><p class=\"eyebrow\">{} · {}</p><h3>{}</h3><pre>{}</pre></article>",
            escape(&constraint.id),
            escape(&constraint.id),
            escape(&constraint.scope),
            escape(&constraint.title),
            escape(&constraint.canonical),
        )
        .unwrap();
    }
    out.push_str("</section><section><h2>Implementations</h2><ul>");
    for path in &intent.implements {
        write!(
            out,
            "<li><code title=\"{}\">{}</code></li>",
            escape(path),
            escape(path),
        )
        .unwrap();
    }
    out.push_str("</ul></section><section><h2>Scenarios</h2>");
    for scenario in &intent.scenarios {
        write!(
            out,
            "<article id=\"scenario-{}\"><p class=\"eyebrow\">{}</p><h3>{}</h3>",
            escape(&scenario.id),
            escape(&scenario.id),
            escape(&scenario.title),
        )
        .unwrap();
        if scenario.proves.is_empty() {
            out.push_str("<p>Not proved.</p>");
        } else {
            out.push_str("<p>Proved by:</p><ul>");
            for proof in &scenario.proves {
                write!(out, "<li><code>{}</code></li>", escape(proof)).unwrap();
            }
            out.push_str("</ul>");
        }
        out.push_str("</article>");
    }
    out.push_str("</section></article>");
    out
}

fn glossary(snapshot: &ViewSnapshot, current: Page, mode: LinkMode) -> String {
    let mut out =
        String::from("<section><p class=\"eyebrow\">Domain language</p><h1>Glossary</h1>");
    for notion in &snapshot.notions {
        write!(
            out,
            "<article id=\"notion-{}\"><p class=\"eyebrow\">{}</p><h2>{}</h2><p>{}</p><pre>{}</pre><p>Used by: ",
            escape(&notion.name),
            escape(&notion.kind),
            escape(&notion.name),
            escape(&notion.definition),
            escape(&notion.canonical),
        )
        .unwrap();
        let mut first = true;
        for intent in snapshot
            .intents
            .iter()
            .filter(|intent| intent.notions.contains(&notion.name))
        {
            if !first {
                out.push_str(", ");
            }
            first = false;
            write!(
                out,
                "<a href=\"{}\">{}</a>",
                escape(&intent_href(current, &intent.id, mode)),
                escape(&intent.id),
            )
            .unwrap();
        }
        if first {
            out.push_str("none");
        }
        out.push_str(".</p></article>");
    }
    out.push_str("</section>");
    out
}

fn coverage(snapshot: &ViewSnapshot) -> String {
    let mut out = format!(
        "<section><p class=\"eyebrow\">Spec evidence</p><h1>Coverage</h1><p>{} active of {} intents.</p><div class=\"table-wrap\"><table><thead><tr><th>Subject</th><th>Covered</th><th>Total</th></tr></thead><tbody>",
        snapshot.coverage.intents_active, snapshot.coverage.intents_total
    );
    for row in &snapshot.coverage.rows {
        write!(
            out,
            "<tr><th>{}</th><td>{}</td><td>{}</td></tr>",
            escape(&row.subject),
            row.covered
                .map(|value| value.to_string())
                .unwrap_or_else(|| "—".to_string()),
            row.total,
        )
        .unwrap();
    }
    out.push_str(
        "</tbody></table></div></section><section><h2>Bindings</h2><h3>Implementations</h3><ul>",
    );
    for binding in &snapshot.implementations {
        write!(
            out,
            "<li><code>{}</code> implements <strong>{}</strong></li>",
            escape(&binding.path),
            escape(&binding.intent),
        )
        .unwrap();
    }
    out.push_str("</ul><h3>Proofs</h3><ul>");
    for binding in &snapshot.proofs {
        write!(
            out,
            "<li><code>{}</code> proves <strong>{}</strong></li>",
            escape(&binding.test),
            escape(&binding.scenario),
        )
        .unwrap();
    }
    out.push_str("</ul></section><section><h2>Constraints</h2><ul>");
    for constraint in &snapshot.constraints {
        write!(
            out,
            "<li><strong>{}</strong> {} · {} · {}</li>",
            escape(&constraint.id),
            escape(&constraint.title),
            escape(&constraint.kind),
            escape(&constraint.scope),
        )
        .unwrap();
    }
    out.push_str("</ul></section>");
    out
}

fn graph_ref(
    snapshot: &ViewSnapshot,
    node: Option<&super::model::GraphNodeView>,
    id: &str,
    current: Page,
    mode: LinkMode,
) -> String {
    let Some(node) = node else {
        return format!("<code>{}</code>", escape(id));
    };
    let link = match node.kind.as_str() {
        "intent" => Some(intent_href(current, &node.id, mode)),
        "scenario" => snapshot
            .scenarios
            .iter()
            .find(|scenario| scenario.id == node.id)
            .map(|scenario| {
                format!(
                    "{}#scenario-{}",
                    intent_href(current, &scenario.intent, mode),
                    scenario.id
                )
            }),
        "notion" => Some(format!(
            "{}#notion-{}",
            href(current, PageTarget::Glossary, mode),
            node.id
        )),
        _ => None,
    };
    match link {
        Some(link) => format!("<a href=\"{}\">{}</a>", escape(&link), escape(&node.id)),
        None => format!("<code>{}</code>", escape(&node.id)),
    }
}

fn nav_link(label: &str, current: Page, target: PageTarget<'_>, mode: LinkMode) -> String {
    format!(
        "<a href=\"{}\">{}</a>",
        escape(&href(current, target, mode)),
        label
    )
}

#[derive(Clone, Copy)]
enum PageTarget<'a> {
    Dashboard,
    Graph,
    Intent(&'a str),
    Glossary,
    Coverage,
}

fn intent_href(current: Page, id: &str, mode: LinkMode) -> String {
    href(current, PageTarget::Intent(id), mode)
}

fn href(current: Page, target: PageTarget<'_>, mode: LinkMode) -> String {
    match mode {
        LinkMode::Server => match target {
            PageTarget::Dashboard => "/".to_string(),
            PageTarget::Graph => "/graph".to_string(),
            PageTarget::Intent(id) => format!("/intent/{id}"),
            PageTarget::Glossary => "/glossary".to_string(),
            PageTarget::Coverage => "/coverage".to_string(),
        },
        LinkMode::Export => {
            let nested = matches!(current, Page::Intent(_));
            match target {
                PageTarget::Dashboard => root_file("index.html", nested),
                PageTarget::Graph => root_file("graph.html", nested),
                PageTarget::Intent(id) if nested => format!("{id}.html"),
                PageTarget::Intent(id) => format!("intents/{id}.html"),
                PageTarget::Glossary => root_file("glossary.html", nested),
                PageTarget::Coverage => root_file("coverage.html", nested),
            }
        }
    }
}

fn root_file(file: &str, nested: bool) -> String {
    if nested {
        format!("../{file}")
    } else {
        file.to_string()
    }
}

fn escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

const CSS: &str = "
:root{color-scheme:light;--ink:#152018;--muted:#5b675d;--line:#d7ddd8;--paper:#f7f8f4;--accent:#1f6f4a}*{box-sizing:border-box}body{margin:0;background:var(--paper);color:var(--ink);font:16px/1.55 ui-sans-serif,system-ui,sans-serif}header,main,footer{width:min(1120px,calc(100% - 32px));margin:auto}header{display:grid;grid-template-columns:auto 1fr auto;gap:24px;align-items:center;padding:24px 0;border-bottom:1px solid var(--line)}nav{display:flex;gap:16px;flex-wrap:wrap}a{color:var(--accent);text-underline-offset:3px}.brand{font-weight:800;font-size:1.25rem}.state{margin:0;text-transform:capitalize}main{padding:48px 0}section+section,article+article{margin-top:40px}h1{font-size:clamp(2rem,5vw,4.5rem);line-height:1;margin:.2em 0}.eyebrow,.pill,small{color:var(--muted);font-size:.78rem;letter-spacing:.08em;text-transform:uppercase}.lede{max-width:62ch;font-size:1.2rem}.metrics,.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(170px,1fr));gap:12px;padding:0;list-style:none}.metrics article,.cards li,article article{border:1px solid var(--line);border-radius:12px;padding:18px;background:#fff}.metrics strong{display:block;font-size:2rem}.cards a{display:flex;gap:8px;flex-direction:column}pre,.table-wrap{overflow:auto;background:#111a14;color:#eef7f0;padding:18px;border-radius:10px}table{width:100%;border-collapse:collapse;background:#fff;color:var(--ink)}th,td{text-align:left;padding:10px;border-bottom:1px solid var(--line)}select{margin-left:8px;padding:8px}.nodes{columns:2;list-style:none;padding:0}.nodes li{break-inside:avoid;padding:6px 0}footer{padding:24px 0 48px;border-top:1px solid var(--line);color:var(--muted)}[hidden]{display:none!important}@media(max-width:720px){header{grid-template-columns:1fr}.nodes{columns:1}}
";

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use telos_core::ids::IntentId;
    use telos_core::state::{ProjectStateKind, StateReport};
    use telos_core::workspace::Workspace;

    use super::{LinkMode, Page, render};
    use crate::view::model::ViewSnapshot;

    fn fixture_snapshot() -> ViewSnapshot {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../telos-core/tests/corpus/billing");
        let workspace = Workspace::discover(&fixture).expect("Billing workspace is discoverable");
        let model = workspace
            .load_model()
            .expect("Billing fixture is a valid model");
        ViewSnapshot::build(
            &StateReport {
                state: ProjectStateKind::Coherent,
                drift: vec![],
                open_changes: vec![],
            },
            &model,
        )
    }

    #[test]
    fn every_page_has_semantic_layout_navigation_and_visible_state() {
        let snapshot = fixture_snapshot();
        let pages = [
            Page::Dashboard,
            Page::Graph,
            Page::Intent(IntentId(42)),
            Page::Glossary,
            Page::Coverage,
        ];

        for page in pages {
            let html = render(&snapshot, page, LinkMode::Server).expect("known page renders");
            assert!(html.starts_with("<!doctype html>"), "{page:?}: {html}");
            assert!(html.contains("<header>"), "{page:?}: {html}");
            assert!(
                html.contains("<nav aria-label=\"Primary\">"),
                "{page:?}: {html}"
            );
            assert!(html.contains("<main>"), "{page:?}: {html}");
            assert!(
                html.contains("Project state: <strong>coherent</strong>"),
                "{page:?}: {html}"
            );
            assert!(html.contains("<style>"), "{page:?}: {html}");
            assert!(!html.contains("http://"), "{page:?}: {html}");
            assert!(!html.contains("https://"), "{page:?}: {html}");
        }
    }

    #[test]
    fn renderer_escapes_model_text_in_text_and_attribute_contexts() {
        let mut snapshot = fixture_snapshot();
        snapshot.intents[1].title = "</script><script>alert(1)</script>".to_string();
        snapshot.intents[1].telos = "&\"<>".to_string();
        snapshot.intents[1].implements = vec!["bad&\"<>.rs".to_string()];

        let html = render(&snapshot, Page::Intent(IntentId(42)), LinkMode::Server).unwrap();

        assert!(
            !html.contains("</script><script>alert(1)</script>"),
            "{html}"
        );
        assert!(
            html.contains("&lt;/script&gt;&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{html}"
        );
        assert!(html.contains("&amp;&quot;&lt;&gt;"), "{html}");
        assert!(
            html.contains("title=\"bad&amp;&quot;&lt;&gt;.rs\""),
            "{html}"
        );
    }

    #[test]
    fn graph_page_has_all_eight_relation_filters_and_seek_safe_rows() {
        let html = render(&fixture_snapshot(), Page::Graph, LinkMode::Server).unwrap();

        assert!(html.contains("<select id=\"relation-filter\""), "{html}");
        for relation in [
            "all",
            "refines",
            "requires",
            "excludes",
            "constrains",
            "verifies",
            "uses",
            "implements",
            "proves",
        ] {
            assert!(
                html.contains(&format!("<option value=\"{relation}\">{relation}</option>")),
                "missing {relation}: {html}"
            );
        }
        assert!(html.contains("data-relation=\"requires\""), "{html}");
        assert!(html.contains("id=\"edge-0\""), "{html}");
        assert!(html.contains("<script>"), "{html}");
        assert!(!html.contains("src=\""), "{html}");
    }

    #[test]
    fn server_links_cover_every_page_family_and_stable_anchor() {
        let snapshot = fixture_snapshot();
        let dashboard = render(&snapshot, Page::Dashboard, LinkMode::Server).unwrap();
        for href in ["/", "/graph", "/intent/INT-0042", "/glossary", "/coverage"] {
            assert!(
                dashboard.contains(&format!("href=\"{href}\"")),
                "{dashboard}"
            );
        }

        let intent = render(&snapshot, Page::Intent(IntentId(42)), LinkMode::Server).unwrap();
        assert!(intent.contains("id=\"intent-INT-0042\""), "{intent}");
        assert!(intent.contains("id=\"scenario-SCN-0107\""), "{intent}");
        assert!(intent.contains("href=\"/intent/INT-0017\""), "{intent}");
        let glossary = render(&snapshot, Page::Glossary, LinkMode::Server).unwrap();
        assert!(glossary.contains("id=\"notion-Invoice\""), "{glossary}");
    }

    #[test]
    fn export_links_are_relative_from_root_and_nested_intent_pages() {
        let snapshot = fixture_snapshot();
        let dashboard = render(&snapshot, Page::Dashboard, LinkMode::Export).unwrap();
        for href in [
            "index.html",
            "graph.html",
            "intents/INT-0042.html",
            "glossary.html",
            "coverage.html",
        ] {
            assert!(
                dashboard.contains(&format!("href=\"{href}\"")),
                "{dashboard}"
            );
        }

        let intent = render(&snapshot, Page::Intent(IntentId(42)), LinkMode::Export).unwrap();
        for href in [
            "../index.html",
            "../graph.html",
            "INT-0017.html",
            "../glossary.html",
            "../coverage.html",
        ] {
            assert!(intent.contains(&format!("href=\"{href}\"")), "{intent}");
        }
    }

    #[test]
    fn unknown_intent_has_no_page() {
        assert_eq!(
            render(
                &fixture_snapshot(),
                Page::Intent(IntentId(9999)),
                LinkMode::Server,
            ),
            None
        );
    }
}
