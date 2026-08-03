#!/usr/bin/env python3
"""Dependency-free structural validation for Telos Agent Skills."""

from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1] / "bundle" / "skills"
EXPECTED = {"telos"}


def validate(skill_dir: Path) -> list[str]:
    errors: list[str] = []
    skill_file = skill_dir / "SKILL.md"
    metadata_file = skill_dir / "agents" / "openai.yaml"
    if not skill_file.is_file():
        return [f"{skill_dir.name}: missing SKILL.md"]
    text = skill_file.read_text(encoding="utf-8")
    match = re.match(r"\A---\n(.*?)\n---\n", text, re.DOTALL)
    if not match:
        errors.append(f"{skill_dir.name}: invalid YAML frontmatter markers")
    else:
        keys = {line.split(":", 1)[0].strip() for line in match.group(1).splitlines() if ":" in line}
        if keys != {"name", "description"}:
            errors.append(f"{skill_dir.name}: frontmatter keys must be name and description")
        if f"name: {skill_dir.name}" not in match.group(1):
            errors.append(f"{skill_dir.name}: name does not match directory")
    if "[TODO" in text or "Structuring This Skill" in text:
        errors.append(f"{skill_dir.name}: unresolved creator placeholder")
    if not metadata_file.is_file():
        errors.append(f"{skill_dir.name}: missing agents/openai.yaml")
    else:
        metadata = metadata_file.read_text(encoding="utf-8")
        if f"${skill_dir.name}" not in metadata:
            errors.append(f"{skill_dir.name}: default_prompt must mention ${skill_dir.name}")
        for key in ("display_name:", "short_description:", "default_prompt:"):
            if key not in metadata:
                errors.append(f"{skill_dir.name}: missing {key}")
    return errors


def main() -> int:
    skill_dirs = [path for path in ROOT.iterdir() if (path / "SKILL.md").is_file()]
    found = {path.name for path in skill_dirs}
    errors = []
    if found != EXPECTED:
        errors.append(f"skill set mismatch: expected {sorted(EXPECTED)}, found {sorted(found)}")
    for path in sorted(skill_dirs):
        errors.extend(validate(path))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Validated {len(found)} Telos Skills.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
