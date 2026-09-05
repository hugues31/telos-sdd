use std::path::Path;

use super::assets::SKILLS;
use super::{
    PlannedWrite, merge_command_hook, merge_owned_block, planned_json, planned_merged_text,
    planned_text, read_optional_object, read_optional_text,
};
use crate::safe_fs::SafeRoot;
use telos_core::error::TelosError;

const START: &str = "<!-- telos-sdd:start -->";
const END: &str = "<!-- telos-sdd:end -->";

const AGENTS_BLOCK: &str = "<!-- telos-sdd:start -->\n\
## Telos workflow\n\
\n\
For every Telos project request, invoke the `telos` skill before reading or editing specification or application files. Never edit paths under `telos/` directly; use the Telos CLI and preserve native human approval prompts.\n\
\n\
Do not rely on the generated Codex guard or rules until setup is reviewed and trusted. Open `/hooks`, review and trust the repository `.codex` layer, and verify the exact `telos agent-guard --host codex` hook before proceeding. Until that review and trust is complete, treat `.codex/hooks.json` and `.codex/rules/telos.rules` as inactive.\n\
<!-- telos-sdd:end -->";

pub(super) const RTK_RULES: &str = include_str!("../../assets/codex-rtk.rules");

const RULES_BLOCK: &str = r#"# telos-sdd:start
# The guard derives current repository context through supported hook messages;
# these static native rules alone own the Codex permission prompts.
prefix_rule(
    pattern = ["telos", "change", "approve"],
    decision = "prompt",
    justification = "Approve only the exact --expected-digest displayed by `telos change diff` immediately before this command",
)

prefix_rule(
    pattern = ["telos", "adopt"],
    decision = "prompt",
    justification = "Adopting the exact --expected-state drift scope is a human decision",
)

prefix_rule(
    pattern = ["telos", "revert"],
    decision = "prompt",
    justification = "Reverting the exact --expected-state drift scope is a human decision",
)
# telos-sdd:end"#;

pub fn plan(root: &SafeRoot) -> Result<Vec<PlannedWrite>, TelosError> {
    let mut writes = Vec::new();
    for (name, content) in SKILLS {
        writes.push(planned_text(
            root,
            &format!(".agents/skills/{name}/SKILL.md"),
            content.to_string(),
        )?);
    }

    let agents = read_optional_text(root, "AGENTS.md")?;
    writes.push(planned_merged_text(
        "AGENTS.md",
        merge_owned_block(&agents.value, START, END, AGENTS_BLOCK)?,
        agents.initial,
    ));

    let mut hooks = read_optional_object(root, ".codex/hooks.json")?;
    merge_command_hook(
        &mut hooks.value,
        super::matcher(super::AgentHost::Codex),
        super::guard_command(super::AgentHost::Codex),
    )?;
    writes.push(planned_json(
        ".codex/hooks.json",
        &hooks.value,
        hooks.initial,
    )?);

    let rules = read_optional_text(root, ".codex/rules/telos.rules")?;
    let merged = merge_owned_block(
        &rules.value,
        "# telos-sdd:start",
        "# telos-sdd:end",
        &RULES_BLOCK.replace(
            "# telos-sdd:end",
            &format!("{}# telos-sdd:end", RTK_RULES.replace("\r\n", "\n")),
        ),
    )?;
    writes.push(planned_merged_text(
        ".codex/rules/telos.rules",
        merged,
        rules.initial,
    ));
    Ok(writes)
}

/// A new guard must not assume an older installation has the RTK prompts.
/// Recognize only our shipped block; custom rules must use the direct route.
pub(super) fn rtk_rules_installed(root: &Path) -> bool {
    let Ok(root) = SafeRoot::open(root) else {
        return false;
    };
    let Ok(Some(bytes)) = root.read_optional(Path::new(".codex/rules/telos.rules")) else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    text.replace("\r\n", "\n")
        .contains(&RTK_RULES.replace("\r\n", "\n"))
}
