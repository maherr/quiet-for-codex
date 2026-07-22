#!/usr/bin/env python3
"""Prepare manifest-pinned rusty_v8 archives and generated bindings for Cargo."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import tarfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.request import Request
from urllib.request import urlopen


SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_MANIFEST = SCRIPT_DIR / "v8-notices-manifest.json"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
DOWNLOAD_TIMEOUT_SECS = 180


class ArtifactError(RuntimeError):
    """Raised when a pinned rusty_v8 input is missing or fails verification."""


@dataclass(frozen=True)
class PreparedArtifacts:
    archive: Path
    binding: Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Download and verify one manifest-pinned rusty_v8 artifact pair."
    )
    parser.add_argument("--target", required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--github-env", type=Path)
    return parser.parse_args()


def sha256_path(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_string(mapping: dict[str, Any], key: str, *, context: str) -> str:
    value = mapping.get(key)
    if not isinstance(value, str) or not value:
        raise ArtifactError(f"{context}.{key} must be a non-empty string")
    return value


def require_sha256(mapping: dict[str, Any], key: str, *, context: str) -> str:
    value = require_string(mapping, key, context=context)
    if SHA256_RE.fullmatch(value) is None:
        raise ArtifactError(f"{context}.{key} must be a lowercase SHA-256 digest")
    return value


def load_manifest(path: Path) -> dict[str, Any]:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ArtifactError(f"Could not read V8 manifest {path}: {exc}") from exc
    if not isinstance(manifest, dict) or manifest.get("schemaVersion") != 1:
        raise ArtifactError("Unsupported V8 manifest schema")
    return manifest


def download_verified(url: str, destination: Path, expected_sha256: str) -> Path:
    if destination.is_file() and sha256_path(destination) == expected_sha256:
        return destination

    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.{os.getpid()}.tmp")
    temporary.unlink(missing_ok=True)
    try:
        request = Request(url, headers={"User-Agent": "codex-quiet-release"})
        with urlopen(request, timeout=DOWNLOAD_TIMEOUT_SECS) as response:
            with temporary.open("wb") as output:
                shutil.copyfileobj(response, output)
        actual = sha256_path(temporary)
        if actual != expected_sha256:
            raise ArtifactError(
                f"SHA-256 mismatch for {url}: expected {expected_sha256}, found {actual}"
            )
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)
    return destination


def extract_verified_binding(
    crate_archive: Path,
    *,
    version: str,
    member_path: str,
    destination: Path,
    expected_sha256: str,
) -> Path:
    if destination.is_file() and sha256_path(destination) == expected_sha256:
        return destination
    if Path(member_path).is_absolute() or ".." in Path(member_path).parts:
        raise ArtifactError(f"Unsafe Rust crate binding path: {member_path!r}")

    archive_member = f"v8-{version}/{member_path}"
    try:
        with tarfile.open(crate_archive, "r:gz") as crate:
            member = crate.getmember(archive_member)
            if not member.isfile():
                raise ArtifactError(
                    f"Rust crate binding member is not a file: {archive_member}"
                )
            extracted = crate.extractfile(member)
            if extracted is None:
                raise ArtifactError(
                    f"Could not extract Rust crate binding: {archive_member}"
                )
            content = extracted.read()
    except (KeyError, tarfile.TarError, OSError) as exc:
        raise ArtifactError(
            f"Could not read binding {archive_member} from {crate_archive}: {exc}"
        ) from exc

    actual = hashlib.sha256(content).hexdigest()
    if actual != expected_sha256:
        raise ArtifactError(
            f"SHA-256 mismatch for {archive_member}: "
            f"expected {expected_sha256}, found {actual}"
        )
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.{os.getpid()}.tmp")
    temporary.write_bytes(content)
    temporary.replace(destination)
    return destination


def prepare_artifacts(
    manifest_path: Path, target: str, output_dir: Path
) -> PreparedArtifacts:
    manifest = load_manifest(manifest_path)
    locked = manifest.get("lockedInputs")
    if not isinstance(locked, dict):
        raise ArtifactError("lockedInputs must be an object")
    artifacts = locked.get("artifacts")
    if not isinstance(artifacts, list):
        raise ArtifactError("lockedInputs.artifacts must be an array")
    matches = [
        artifact
        for artifact in artifacts
        if isinstance(artifact, dict) and artifact.get("target") == target
    ]
    if len(matches) != 1:
        raise ArtifactError(
            f"Expected exactly one pinned rusty_v8 artifact for {target}, found {len(matches)}"
        )
    artifact = matches[0]
    context = f"rusty_v8 artifact {target}"
    archive_url = require_string(artifact, "url", context=context)
    archive_sha256 = require_sha256(artifact, "sha256", context=context)
    binding_sha256 = require_sha256(artifact, "bindingSha256", context=context)

    output_dir.mkdir(parents=True, exist_ok=True)
    archive = download_verified(
        archive_url,
        output_dir / f"rusty_v8_archive_{target}.gz",
        archive_sha256,
    )
    binding = output_dir / f"src_binding_release_{target}.rs"
    binding_url = artifact.get("bindingUrl")
    binding_crate_path = artifact.get("bindingCratePath")
    if isinstance(binding_url, str) and binding_url and binding_crate_path is None:
        download_verified(binding_url, binding, binding_sha256)
    elif (
        isinstance(binding_crate_path, str)
        and binding_crate_path
        and binding_url is None
    ):
        rust_crate = locked.get("rustCrate")
        if not isinstance(rust_crate, dict):
            raise ArtifactError("lockedInputs.rustCrate must be an object")
        crate_version = require_string(
            rust_crate, "version", context="lockedInputs.rustCrate"
        )
        crate_url = require_string(rust_crate, "url", context="lockedInputs.rustCrate")
        crate_sha256 = require_sha256(
            rust_crate, "checksum", context="lockedInputs.rustCrate"
        )
        crate_archive = download_verified(
            crate_url,
            output_dir / f"v8-{crate_version}.crate",
            crate_sha256,
        )
        extract_verified_binding(
            crate_archive,
            version=crate_version,
            member_path=binding_crate_path,
            destination=binding,
            expected_sha256=binding_sha256,
        )
    else:
        raise ArtifactError(
            f"{context} must define exactly one of bindingUrl or bindingCratePath"
        )

    return PreparedArtifacts(archive=archive.resolve(), binding=binding.resolve())


def append_github_env(path: Path, artifacts: PreparedArtifacts) -> None:
    with path.open("a", encoding="utf-8", newline="\n") as output:
        output.write(f"RUSTY_V8_ARCHIVE={artifacts.archive}\n")
        output.write(f"RUSTY_V8_SRC_BINDING_PATH={artifacts.binding}\n")


def main() -> int:
    args = parse_args()
    artifacts = prepare_artifacts(args.manifest, args.target, args.output_dir)
    if args.github_env is not None:
        append_github_env(args.github_env, artifacts)
    print(f"Prepared pinned rusty_v8 archive: {artifacts.archive}")
    print(f"Prepared pinned rusty_v8 binding: {artifacts.binding}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
