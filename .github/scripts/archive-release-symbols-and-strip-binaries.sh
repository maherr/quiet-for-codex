#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: archive-release-symbols-and-strip-binaries.sh \
  --target <rust-target> \
  --artifact-name <artifact-name> \
  --release-dir <dir> \
  --archive-dir <dir> \
  --binaries "<space-delimited binary basenames>"
EOF
}

target=""
artifact_name=""
release_dir=""
archive_dir=""
binaries=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      target="${2:?--target requires a value}"
      shift 2
      ;;
    --artifact-name)
      artifact_name="${2:?--artifact-name requires a value}"
      shift 2
      ;;
    --release-dir)
      release_dir="${2:?--release-dir requires a value}"
      shift 2
      ;;
    --archive-dir)
      archive_dir="${2:?--archive-dir requires a value}"
      shift 2
      ;;
    --binaries)
      binaries="${2:?--binaries requires a value}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unexpected argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$target" || -z "$artifact_name" || -z "$release_dir" || -z "$archive_dir" || -z "$binaries" ]]; then
  usage >&2
  exit 1
fi

symbols_root="${RUNNER_TEMP:-/tmp}/codex-symbols-${artifact_name}"
symbols_dir="${symbols_root}/codex-symbols-${artifact_name}"
archive_path="${archive_dir%/}/codex-symbols-${artifact_name}.tar.gz"
rm -rf "$symbols_root"
mkdir -p "$symbols_dir" "$archive_dir"
read -r -a binary_names <<< "$binaries"
manifest_path="${symbols_dir}/BUILD_IDS.txt"
{
  printf 'artifact=%s\n' "$artifact_name"
  printf 'target=%s\n' "$target"
  printf 'source=%s\n' "${GITHUB_SHA:-unknown}"
} > "$manifest_path"

file_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

case "$target" in
  *apple-darwin)
    for binary in "${binary_names[@]}"; do
      binary_path="${release_dir%/}/${binary}"
      dsym_path="${binary_path}.dSYM"
      if [[ ! -f "$binary_path" ]]; then
        echo "Binary $binary_path not found" >&2
        exit 1
      fi
      if [[ ! -d "$dsym_path" ]]; then
        dsymutil "$binary_path" -o "$dsym_path"
      fi

      cp -RL "$dsym_path" "${symbols_dir}/${binary}.dSYM"
      binary_uuid="$(dwarfdump --uuid "$binary_path" | awk '{print $2}' | sort | tr '\n' ',')"
      symbols_uuid="$(dwarfdump --uuid "${symbols_dir}/${binary}.dSYM" | awk '{print $2}' | sort | tr '\n' ',')"
      if [[ -z "$binary_uuid" || "$binary_uuid" != "$symbols_uuid" ]]; then
        echo "dSYM UUID does not match $binary_path" >&2
        exit 1
      fi
      strip -S -x "$binary_path"
      printf '%s uuid=%s shipped_binary_sha256=%s\n' \
        "$binary" "${binary_uuid%,}" "$(file_sha256 "$binary_path")" >> "$manifest_path"
    done
    ;;
  *linux*)
    objcopy_bin="${OBJCOPY:-objcopy}"
    strip_bin="${STRIP:-strip}"
    for binary in "${binary_names[@]}"; do
      binary_path="${release_dir%/}/${binary}"
      debug_path="${symbols_dir}/${binary}.debug"
      if [[ ! -f "$binary_path" ]]; then
        echo "Binary $binary_path not found" >&2
        exit 1
      fi

      "$objcopy_bin" --only-keep-debug "$binary_path" "$debug_path"
      binary_build_id="$(readelf -n "$binary_path" | awk '/Build ID:/ {print $3; exit}')"
      debug_build_id="$(readelf -n "$debug_path" | awk '/Build ID:/ {print $3; exit}')"
      if [[ -z "$binary_build_id" || "$binary_build_id" != "$debug_build_id" ]]; then
        echo "Debug sidecar build ID does not match $binary_path" >&2
        exit 1
      fi
      debug_sha256="$(file_sha256 "$debug_path")"
      "$strip_bin" --strip-debug --strip-unneeded "$binary_path"
      "$objcopy_bin" --add-gnu-debuglink="$debug_path" "$binary_path"
      printf '%s build_id=%s shipped_binary_sha256=%s debug_sha256=%s\n' \
        "$binary" "$binary_build_id" "$(file_sha256 "$binary_path")" \
        "$debug_sha256" >> "$manifest_path"
    done
    ;;
  *windows*)
    for binary in "${binary_names[@]}"; do
      binary_path="${release_dir%/}/${binary}.exe"
      pdb_path="${release_dir%/}/${binary}.pdb"
      if [[ ! -f "$binary_path" ]]; then
        echo "Binary $binary_path not found" >&2
        exit 1
      fi
      if [[ ! -f "$pdb_path" ]]; then
        echo "PDB $pdb_path not found" >&2
        exit 1
      fi

      cp "$pdb_path" "${symbols_dir}/${binary}.pdb"
      printf '%s binary_sha256=%s pdb_sha256=%s\n' \
        "$binary" "$(file_sha256 "$binary_path")" "$(file_sha256 "$pdb_path")" \
        >> "$manifest_path"
    done
    ;;
  *)
    echo "No symbols packaging support for target: $target" >&2
    exit 1
    ;;
esac

rm -f "$archive_path"
tar -C "$symbols_root" -czf "$archive_path" "codex-symbols-${artifact_name}"
