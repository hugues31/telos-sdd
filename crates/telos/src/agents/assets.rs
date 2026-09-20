//! Canonical skill assets. Both host renderers write these exact bytes.

pub const SKILLS: [(&str, &str); 4] = [
    ("telos", include_str!("../../assets/skills/telos/SKILL.md")),
    (
        "telos-challenger",
        include_str!("../../assets/skills/telos-challenger/SKILL.md"),
    ),
    (
        "telos-implementer",
        include_str!("../../assets/skills/telos-implementer/SKILL.md"),
    ),
    (
        "telos-brainstormer",
        include_str!("../../assets/skills/telos-brainstormer/SKILL.md"),
    ),
];
