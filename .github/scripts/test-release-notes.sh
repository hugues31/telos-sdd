#!/usr/bin/env bash
set -euo pipefail

git_cliff="${1:-git-cliff}"
repo_root="$(git rev-parse --show-toplevel)"
config="${repo_root}/.github/cliff.toml"
workflow="${repo_root}/.github/workflows/release.yml"
ci_workflow="${repo_root}/.github/workflows/ci.yml"
fixture="$(mktemp -d)"
notes="${fixture}/release-notes.md"
trap 'rm -rf "${fixture}"' EXIT

if [[ ! -f "${config}" ]]; then
  echo "missing release notes configuration: ${config}" >&2
  exit 1
fi

git -C "${fixture}" init --quiet --initial-branch=main
git -C "${fixture}" config user.name "Release Notes Test"
git -C "${fixture}" config user.email "release-notes@example.com"

git -C "${fixture}" commit --quiet --allow-empty -m "chore: initialize fixture"
git -C "${fixture}" switch --quiet --create side-release
git -C "${fixture}" switch --quiet main
git -C "${fixture}" commit --quiet --allow-empty -m "feat(core): land before the previous release"
git -C "${fixture}" tag --annotate v1.0.0 --message "v1.0.0"
git -C "${fixture}" switch --quiet side-release
git -C "${fixture}" commit --quiet --allow-empty -m "feat(side): stay off the release branch"
git -C "${fixture}" tag --annotate v9.9.9 --message "v9.9.9"
git -C "${fixture}" switch --quiet main
git -C "${fixture}" commit --quiet --allow-empty -m "feat(view): add graph search"
git -C "${fixture}" tag --annotate preview-v8.0.0 --message "not a release tag"
git -C "${fixture}" commit --quiet --allow-empty -m "fix: preserve Windows paths"
git -C "${fixture}" commit --quiet --allow-empty -m "docs: explain release behavior"
git -C "${fixture}" commit --quiet --allow-empty -m "ci: verify the release pipeline"
git -C "${fixture}" commit --quiet --allow-empty -m "feat(api)!: remove the legacy endpoint"
git -C "${fixture}" commit --quiet --allow-empty \
  -m "fix(parser): reject ambiguous input" \
  -m "BREAKING CHANGE: ambiguous input is no longer accepted"
git -C "${fixture}" commit --quiet --allow-empty -m "chore(release)!: prepare v1.1.0"
git -C "${fixture}" tag --annotate v1.1.0 --message "v1.1.0"
git -C "${fixture}" checkout --quiet --detach v1.1.0

"${git_cliff}" \
  --config "${config}" \
  --repository "${fixture}" \
  --current \
  --strip header \
  --output "${notes}"

grep -Fq "### Breaking Changes" "${notes}"
grep -Fq "Remove the legacy endpoint" "${notes}"
grep -Fq "Reject ambiguous input" "${notes}"
grep -Fq "### Features" "${notes}"
grep -Fq "Add graph search" "${notes}"
grep -Fq "### Bug Fixes" "${notes}"
grep -Fq "Preserve Windows paths" "${notes}"
grep -Fq "### Documentation" "${notes}"
grep -Fq "Explain release behavior" "${notes}"
grep -Fq "### Maintenance" "${notes}"
grep -Fq "Verify the release pipeline" "${notes}"

if grep -Fq "### Continuous Integration" "${notes}"; then
  echo "release notes did not collapse CI commits into maintenance" >&2
  exit 1
fi

if grep -Fq "Initialize fixture" "${notes}"; then
  echo "release notes included commits from before the previous tag" >&2
  exit 1
fi

if grep -Fq "Land before the previous release" "${notes}"; then
  echo "release notes used a side-branch tag as the previous release" >&2
  exit 1
fi

if grep -Fq "Stay off the release branch" "${notes}"; then
  echo "release notes included commits from a side-branch tag" >&2
  exit 1
fi

if grep -Fq "Prepare v1.1.0" "${notes}"; then
  echo "release notes included the release preparation commit" >&2
  exit 1
fi

assert_file_contains() {
  local file="$1"
  local expected="$2"
  if ! grep -Fq -- "${expected}" "${file}"; then
    echo "${file} is missing: ${expected}" >&2
    exit 1
  fi
}

action_ref="orhun/git-cliff-action@f50e11560dce63f7c33227798f90b924471a88b5"
assert_file_contains "${workflow}" "fetch-depth: 0"
assert_file_contains "${workflow}" "uses: ${action_ref}"
assert_file_contains "${workflow}" 'version: "v2.13.1"'
assert_file_contains "${workflow}" "config: .github/cliff.toml"
assert_file_contains "${workflow}" "args: --current --strip header"
assert_file_contains "${workflow}" "OUTPUT: release-notes.md"
assert_file_contains "${workflow}" 'gh release edit "$TAG" --notes-file release-notes.md'
assert_file_contains "${workflow}" 'gh release create "$TAG" dist/* --notes-file release-notes.md --verify-tag'
assert_file_contains "${ci_workflow}" "uses: ${action_ref}"
assert_file_contains "${ci_workflow}" "Test conventional release notes"
assert_file_contains "${ci_workflow}" '.github/scripts/test-release-notes.sh "$RUNNER_TEMP/git-cliff/bin/git-cliff"'

if grep -Fq -- "--generate-notes" "${workflow}"; then
  echo "release workflow still uses GitHub's generic generated notes" >&2
  exit 1
fi
