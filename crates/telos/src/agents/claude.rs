use std::path::Path;

use super::assets::SKILLS;
use super::{merge_command_hook, read_optional_object, write_json, write_text};
use telos_core::error::TelosError;

pub fn render(root: &Path) -> Result<(), TelosError> {
    for (name, content) in SKILLS {
        write_text(
            &root.join(format!(".claude/skills/{name}/SKILL.md")),
            content,
        )?;
    }

    let path = root.join(".claude/settings.json");
    let mut settings = read_optional_object(&path)?;
    merge_command_hook(
        &mut settings,
        super::AgentHost::Claude.matcher(),
        super::AgentHost::Claude.guard_command(),
    )?;
    write_json(&path, &settings)
}
