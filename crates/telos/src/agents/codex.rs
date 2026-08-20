use std::path::Path;

use super::assets::SKILLS;
use super::{
    merge_command_hook, merge_owned_block, read_optional_object, read_optional_text, write_json,
    write_text,
};
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

const RULES_BLOCK: &str = r#"# telos-sdd:start
# The skill prints the current digest immediately before these static native prompts.
prefix_rule(
    pattern = ["telos", "change", "approve"],
    decision = "prompt",
    justification = "Approve only the digest displayed by `telos change diff` immediately before this command",
)

prefix_rule(
    pattern = ["telos", "adopt"],
    decision = "prompt",
    justification = "Adopting drift is a human decision",
)

prefix_rule(
    pattern = ["telos", "revert"],
    decision = "prompt",
    justification = "Reverting drift is a human decision",
)
# telos-sdd:end"#;

pub fn render(root: &Path) -> Result<(), TelosError> {
    for (name, content) in SKILLS {
        write_text(
            &root.join(format!(".agents/skills/{name}/SKILL.md")),
            content,
        )?;
    }

    let agents_path = root.join("AGENTS.md");
    let agents = read_optional_text(&agents_path)?;
    write_text(
        &agents_path,
        &merge_owned_block(&agents, START, END, AGENTS_BLOCK),
    )?;

    let hooks_path = root.join(".codex/hooks.json");
    let mut hooks = read_optional_object(&hooks_path)?;
    merge_command_hook(
        &mut hooks,
        super::matcher(super::AgentHost::Codex),
        super::guard_command(super::AgentHost::Codex),
    )?;
    write_json(&hooks_path, &hooks)?;

    let rules_path = root.join(".codex/rules/telos.rules");
    let rules = read_optional_text(&rules_path)?;
    let merged = merge_owned_block(&rules, "# telos-sdd:start", "# telos-sdd:end", RULES_BLOCK);
    write_text(&rules_path, &merged)
}
