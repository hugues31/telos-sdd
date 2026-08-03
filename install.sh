#!/bin/sh
set -eu

repo="hugues31/telos-sdd"
version="${TELOS_VERSION:-}"
install_dir="${TELOS_INSTALL_DIR:-${HOME}/.local/bin}"

if [ -z "$version" ]; then
  version=$(curl -fsSL "https://api.github.com/repos/${repo}/releases/latest" | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
fi
if [ -z "$version" ]; then
  echo "Could not resolve the latest Telos release." >&2
  exit 1
fi

case "$(uname -s)" in
  Linux) os="linux" ;;
  Darwin) os="darwin" ;;
  *) echo "Unsupported operating system: $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64) arch="amd64" ;;
  arm64|aarch64) arch="arm64" ;;
  *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

plain_version=${version#v}
archive="telos_${plain_version}_${os}_${arch}.tar.gz"
base_url="https://github.com/${repo}/releases/download/${version}"
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT INT TERM

curl -fsSL "${base_url}/${archive}" -o "${tmp_dir}/${archive}"
curl -fsSL "${base_url}/checksums.txt" -o "${tmp_dir}/checksums.txt"

expected=$(awk -v name="$archive" '$2 == name { print $1 }' "${tmp_dir}/checksums.txt")
if [ -z "$expected" ]; then
  echo "Release checksum does not contain ${archive}." >&2
  exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "${tmp_dir}/${archive}" | awk '{print $1}')
else
  actual=$(shasum -a 256 "${tmp_dir}/${archive}" | awk '{print $1}')
fi
if [ "$actual" != "$expected" ]; then
  echo "Checksum verification failed for ${archive}." >&2
  exit 1
fi

tar -xzf "${tmp_dir}/${archive}" -C "$tmp_dir"
mkdir -p "$install_dir"
install -m 0755 "${tmp_dir}/telos" "${install_dir}/telos"
echo "Installed Telos ${version} to ${install_dir}/telos"

