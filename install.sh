#!/bin/sh
set -eu

repository="rbas/forkstr"
install_dir="${FORKSTR_INSTALL_DIR:-/usr/local/bin}"

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) target="x86_64-unknown-linux-gnu" ;;
    Darwin-x86_64) target="x86_64-apple-darwin" ;;
    Darwin-arm64) target="aarch64-apple-darwin" ;;
    *)
        echo "forkstr: no pre-built binary for $(uname -s) $(uname -m)" >&2
        exit 1
        ;;
esac

if ! command -v curl >/dev/null 2>&1; then
    echo "forkstr: curl is required" >&2
    exit 1
fi

version="${FORKSTR_VERSION:-}"
if [ -z "${version}" ]; then
    version="$(
        curl --proto '=https' --tlsv1.2 -fsSL \
            "https://api.github.com/repos/${repository}/releases/latest" |
            sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p'
    )"
fi
if [ -z "${version}" ]; then
    echo "forkstr: could not determine the latest release" >&2
    exit 1
fi

archive="forkstr-${version}-${target}.tar.gz"
release_url="https://github.com/${repository}/releases/download/${version}"
temporary_directory="$(mktemp -d "${TMPDIR:-/tmp}/forkstr.XXXXXX")"
cleanup() {
    rm -rf "${temporary_directory}"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

echo "Downloading forkstr ${version} for ${target}..."
curl --proto '=https' --tlsv1.2 -fsSL \
    "${release_url}/${archive}" -o "${temporary_directory}/${archive}"
curl --proto '=https' --tlsv1.2 -fsSL \
    "${release_url}/SHA256SUMS" -o "${temporary_directory}/SHA256SUMS"

expected="$(
    awk -v archive="${archive}" '$2 == archive { print $1; exit }' \
        "${temporary_directory}/SHA256SUMS"
)"
if [ -z "${expected}" ]; then
    echo "forkstr: ${archive} is missing from SHA256SUMS" >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${temporary_directory}/${archive}" | sed 's/[[:space:]].*//')"
elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "${temporary_directory}/${archive}" | sed 's/[[:space:]].*//')"
else
    echo "forkstr: sha256sum or shasum is required to verify the download" >&2
    exit 1
fi

if [ "${actual}" != "${expected}" ]; then
    echo "forkstr: checksum verification failed for ${archive}" >&2
    exit 1
fi

tar -xzf "${temporary_directory}/${archive}" -C "${temporary_directory}"
if [ ! -f "${temporary_directory}/forkstr" ]; then
    echo "forkstr: release archive did not contain the forkstr binary" >&2
    exit 1
fi

if mkdir -p "${install_dir}" 2>/dev/null && [ -w "${install_dir}" ]; then
    install -m 755 "${temporary_directory}/forkstr" "${install_dir}/forkstr"
elif command -v sudo >/dev/null 2>&1; then
    sudo install -d "${install_dir}"
    sudo install -m 755 "${temporary_directory}/forkstr" "${install_dir}/forkstr"
else
    echo "forkstr: cannot write to ${install_dir}; set FORKSTR_INSTALL_DIR to a writable directory" >&2
    exit 1
fi

echo "Installed forkstr ${version} to ${install_dir}/forkstr"
