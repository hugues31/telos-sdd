use std::path::Path;

use super::assets::SKILLS;
use super::{PlannedWrite, merge_command_hook, planned_json, planned_text, read_optional_object};
use telos_core::error::TelosError;

pub fn plan(root: &Path) -> Result<Vec<PlannedWrite>, TelosError> {
    let mut writes = Vec::new();
    for (name, content) in SKILLS {
        writes.push(planned_text(
            root,
            &format!(".claude/skills/{name}/SKILL.md"),
            content.to_string(),
        )?);
    }

    let mut settings = read_optional_object(&root.join(".claude/settings.json"))?;
    merge_command_hook(
        &mut settings,
        super::matcher(super::AgentHost::Claude),
        super::guard_command(super::AgentHost::Claude),
    )?;
    writes.push(planned_json(root, ".claude/settings.json", &settings)?);
    Ok(writes)
}
