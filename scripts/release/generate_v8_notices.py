#!/usr/bin/env python3
"""Generate the pinned V8 and rusty_v8 third-party notice bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tomllib
from pathlib import Path
from typing import Any


SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[1]
DEFAULT_MANIFEST = SCRIPT_DIR / "v8-notices-manifest.json"
NOTICE_TITLE = "Quiet for Codex V8 and rusty_v8 third-party notices"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
FORBIDDEN_LICENSE_PAYLOADS = (
    b"*** Begin Patch",
    b"*** Add File:",
    b"*** End Patch",
    b"/home/",
)


class NoticeError(RuntimeError):
    """Raised when source pins or vendored notice material have drifted."""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate an offline V8 and rusty_v8 notice bundle."
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, default=REPO_ROOT)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    return parser.parse_args()


def sha256_bytes(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


def safe_relative_path(value: str, *, field: str) -> Path:
    path = Path(value)
    if path.is_absolute() or not path.parts or ".." in path.parts:
        raise NoticeError(f"{field} must be a safe relative path: {value!r}")
    return path


def read_manifest(path: Path) -> dict[str, Any]:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise NoticeError(f"Could not read V8 notice manifest {path}: {exc}") from exc
    if not isinstance(manifest, dict):
        raise NoticeError("V8 notice manifest must be a JSON object")
    if manifest.get("schemaVersion") != 1:
        raise NoticeError("Unsupported V8 notice manifest schemaVersion")
    return manifest


def require_string(mapping: dict[str, Any], key: str, *, context: str) -> str:
    value = mapping.get(key)
    if not isinstance(value, str) or not value.strip():
        raise NoticeError(f"{context}.{key} must be a non-empty string")
    return value


def validate_source_revision(component: dict[str, Any], *, context: str) -> None:
    commit = require_string(component, "commit", context=context)
    source_url = require_string(component, "sourceUrl", context=context)
    if COMMIT_RE.fullmatch(commit) is None:
        raise NoticeError(f"{context}.commit must be a full 40-character commit")
    if commit not in source_url:
        raise NoticeError(f"{context}.sourceUrl must include the exact commit")


def validate_locked_crate(repo_root: Path, manifest: dict[str, Any]) -> None:
    locked_inputs = manifest.get("lockedInputs")
    if not isinstance(locked_inputs, dict):
        raise NoticeError("lockedInputs must be an object")
    crate = locked_inputs.get("rustCrate")
    if not isinstance(crate, dict):
        raise NoticeError("lockedInputs.rustCrate must be an object")
    crate_name = require_string(crate, "name", context="lockedInputs.rustCrate")
    version = require_string(crate, "version", context="lockedInputs.rustCrate")
    checksum = require_string(crate, "checksum", context="lockedInputs.rustCrate")
    crate_url = require_string(crate, "url", context="lockedInputs.rustCrate")
    lock_path = safe_relative_path(
        require_string(crate, "lockFile", context="lockedInputs.rustCrate"),
        field="lockedInputs.rustCrate.lockFile",
    )
    if SHA256_RE.fullmatch(checksum) is None:
        raise NoticeError("lockedInputs.rustCrate.checksum must be SHA-256")
    if version not in crate_url:
        raise NoticeError("lockedInputs.rustCrate.url must include the exact version")

    try:
        lock = tomllib.loads((repo_root / lock_path).read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
        raise NoticeError(f"Could not read Rust lock file {lock_path}: {exc}") from exc
    packages = [
        package
        for package in lock.get("package", [])
        if package.get("name") == crate_name
    ]
    if len(packages) != 1:
        raise NoticeError(
            f"Expected exactly one {crate_name!r} package in {lock_path}, found {len(packages)}"
        )
    actual = packages[0]
    if actual.get("version") != version or actual.get("checksum") != checksum:
        raise NoticeError(
            f"Locked {crate_name} drifted: expected {version} / {checksum}, "
            f"found {actual.get('version')} / {actual.get('checksum')}"
        )


def validate_required_pins(repo_root: Path, manifest: dict[str, Any]) -> None:
    pins = manifest.get("requiredPins")
    if not isinstance(pins, list) or not pins:
        raise NoticeError("requiredPins must be a non-empty array")
    for index, pin in enumerate(pins):
        context = f"requiredPins[{index}]"
        if not isinstance(pin, dict):
            raise NoticeError(f"{context} must be an object")
        relative = safe_relative_path(
            require_string(pin, "path", context=context), field=f"{context}.path"
        )
        literals = pin.get("literals")
        if not isinstance(literals, list) or not literals:
            raise NoticeError(f"{context}.literals must be a non-empty array")
        try:
            content = (repo_root / relative).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as exc:
            raise NoticeError(f"Could not read pinned input {relative}: {exc}") from exc
        for literal in literals:
            if not isinstance(literal, str) or not literal:
                raise NoticeError(f"{context}.literals contains an invalid value")
            if literal not in content:
                raise NoticeError(f"Pinned input drifted: {relative} lacks {literal!r}")


def validate_locked_inputs(manifest: dict[str, Any]) -> None:
    locked_inputs = manifest["lockedInputs"]
    for name in ("rustyV8Source", "v8Source", "unixArtifactBuild"):
        source = locked_inputs.get(name)
        if not isinstance(source, dict):
            raise NoticeError(f"lockedInputs.{name} must be an object")
        validate_source_revision(source, context=f"lockedInputs.{name}")

    artifacts = locked_inputs.get("artifacts")
    if not isinstance(artifacts, list) or not artifacts:
        raise NoticeError("lockedInputs.artifacts must be a non-empty array")
    targets: set[str] = set()
    for index, artifact in enumerate(artifacts):
        context = f"lockedInputs.artifacts[{index}]"
        if not isinstance(artifact, dict):
            raise NoticeError(f"{context} must be an object")
        target = require_string(artifact, "target", context=context)
        if target in targets:
            raise NoticeError(f"Duplicate artifact target: {target}")
        targets.add(target)
        require_string(artifact, "provider", context=context)
        require_string(artifact, "url", context=context)
        digest = require_string(artifact, "sha256", context=context)
        if SHA256_RE.fullmatch(digest) is None:
            raise NoticeError(f"{context}.sha256 must be SHA-256")
        binding_digest = require_string(artifact, "bindingSha256", context=context)
        if SHA256_RE.fullmatch(binding_digest) is None:
            raise NoticeError(f"{context}.bindingSha256 must be SHA-256")
        binding_url = artifact.get("bindingUrl")
        binding_crate_path = artifact.get("bindingCratePath")
        if isinstance(binding_url, str) and binding_url and binding_crate_path is None:
            pass
        elif (
            isinstance(binding_crate_path, str)
            and binding_crate_path
            and binding_url is None
        ):
            safe_relative_path(binding_crate_path, field=f"{context}.bindingCratePath")
        else:
            raise NoticeError(
                f"{context} must define exactly one of bindingUrl or bindingCratePath"
            )


def collect_licenses(
    manifest_path: Path, manifest: dict[str, Any]
) -> list[tuple[dict[str, Any], list[tuple[dict[str, Any], bytes]]]]:
    directory = safe_relative_path(
        require_string(manifest, "licenseDirectory", context="manifest"),
        field="licenseDirectory",
    )
    license_root = manifest_path.parent / directory
    components = manifest.get("components")
    if not isinstance(components, list) or not components:
        raise NoticeError("components must be a non-empty array")

    expected_files: set[Path] = set()
    result: list[tuple[dict[str, Any], list[tuple[dict[str, Any], bytes]]]] = []
    names: set[str] = set()
    for index, component in enumerate(components):
        context = f"components[{index}]"
        if not isinstance(component, dict):
            raise NoticeError(f"{context} must be an object")
        name = require_string(component, "name", context=context)
        if name in names:
            raise NoticeError(f"Duplicate component name: {name}")
        names.add(name)
        require_string(component, "version", context=context)
        require_string(component, "scope", context=context)
        require_string(component, "declaredLicense", context=context)
        validate_source_revision(component, context=context)
        files = component.get("licenseFiles")
        if not isinstance(files, list) or not files:
            raise NoticeError(f"{context}.licenseFiles must be a non-empty array")
        loaded: list[tuple[dict[str, Any], bytes]] = []
        for file_index, license_file in enumerate(files):
            file_context = f"{context}.licenseFiles[{file_index}]"
            if not isinstance(license_file, dict):
                raise NoticeError(f"{file_context} must be an object")
            relative = safe_relative_path(
                require_string(license_file, "path", context=file_context),
                field=f"{file_context}.path",
            )
            if relative in expected_files:
                raise NoticeError(f"Duplicate vendored license path: {relative}")
            expected_files.add(relative)
            source_path = require_string(
                license_file, "sourcePath", context=file_context
            )
            source_url = require_string(license_file, "sourceUrl", context=file_context)
            if source_path not in source_url or component["commit"] not in source_url:
                raise NoticeError(
                    f"{file_context}.sourceUrl must include sourcePath and exact commit"
                )
            digest = require_string(license_file, "sha256", context=file_context)
            if SHA256_RE.fullmatch(digest) is None:
                raise NoticeError(f"{file_context}.sha256 must be SHA-256")
            path = license_root / relative
            try:
                content = path.read_bytes()
            except OSError as exc:
                raise NoticeError(
                    f"Could not read vendored license {path}: {exc}"
                ) from exc
            if not content:
                raise NoticeError(f"Vendored license is empty: {path}")
            for forbidden in FORBIDDEN_LICENSE_PAYLOADS:
                if forbidden in content:
                    raise NoticeError(
                        f"Vendored license contains patch or workspace contamination "
                        f"in {relative}: {forbidden.decode('ascii')!r}"
                    )
            actual_digest = sha256_bytes(content)
            if actual_digest != digest:
                raise NoticeError(
                    f"Vendored license hash drifted for {relative}: "
                    f"expected {digest}, found {actual_digest}"
                )
            try:
                content.decode("utf-8")
            except UnicodeDecodeError as exc:
                raise NoticeError(f"Vendored license is not UTF-8: {path}") from exc
            loaded.append((license_file, content))
        result.append((component, loaded))

    actual_files = {
        path.relative_to(license_root)
        for path in license_root.rglob("*")
        if path.is_file()
    }
    if actual_files != expected_files:
        missing = sorted(str(path) for path in expected_files - actual_files)
        extra = sorted(str(path) for path in actual_files - expected_files)
        raise NoticeError(
            f"Vendored V8 license set drifted; missing={missing}, extra={extra}"
        )
    return result


def render_notice(
    manifest: dict[str, Any],
    components: list[tuple[dict[str, Any], list[tuple[dict[str, Any], bytes]]]],
) -> str:
    locked = manifest["lockedInputs"]
    crate = locked["rustCrate"]
    rusty = locked["rustyV8Source"]
    v8 = locked["v8Source"]
    build = locked["unixArtifactBuild"]
    lines = [
        NOTICE_TITLE,
        "=" * len(NOTICE_TITLE),
        "",
        "Manifest schema: 1",
        "",
        "About this file",
        "---------------",
        "This file is generated offline from a checked-in manifest and checked-in",
        "license texts. The generator verifies the exact Rust lock entry, source and",
        "build pins, the complete vendored file set, and every license-text SHA-256.",
        "",
        "The inventory is provided to preserve notices and source provenance. It is",
        "not legal advice or a legal determination that every possible obligation has",
        "been identified. Some V8 source-tree notices are included conservatively even",
        "when the corresponding test or build component may not be linked at runtime.",
        "",
        "Locked provenance",
        "-----------------",
        f"Rust crate: {crate['name']} {crate['version']}",
        f"Rust crate checksum: {crate['checksum']}",
        f"Rust crate archive: {crate['url']}",
        f"rusty_v8 tag: {rusty['tag']}",
        f"rusty_v8 commit: {rusty['commit']}",
        f"rusty_v8 source: {rusty['sourceUrl']}",
        f"V8 tag: {v8['tag']}",
        f"V8 commit: {v8['commit']}",
        f"V8 source: {v8['sourceUrl']}",
        f"V8 source archive integrity: {v8['archiveIntegrity']}",
        f"Unix artifact build tag: {build['tag']}",
        f"Unix artifact build commit: {build['commit']}",
        f"Unix artifact source: {build['sourceUrl']}",
        f"Unix artifact release: {build['releaseUrl']}",
        "",
        "Pinned release artifacts",
        "------------------------",
    ]
    for artifact in locked["artifacts"]:
        lines.extend(
            [
                f"Target: {artifact['target']}",
                f"  Provider: {artifact['provider']}",
                f"  URL: {artifact['url']}",
                f"  Archive SHA-256: {artifact['sha256']}",
            ]
        )
        if "bindingUrl" in artifact:
            lines.append(f"  Binding URL: {artifact['bindingUrl']}")
        else:
            lines.append(f"  Binding Rust crate path: {artifact['bindingCratePath']}")
        lines.append(f"  Binding SHA-256: {artifact['bindingSha256']}")

    lines.extend(["", "License texts", "-------------", ""])
    for component, license_files in components:
        lines.extend(
            [
                "=" * 79,
                f"Component: {component['name']}",
                f"Version: {component['version']}",
                f"Scope: {component['scope']}",
                f"Declared license: {component['declaredLicense']}",
                f"Source commit: {component['commit']}",
                f"Source: {component['sourceUrl']}",
                "License source files:",
            ]
        )
        for license_file, _ in license_files:
            lines.append(f"  {license_file['sourcePath']}: {license_file['sourceUrl']}")
        lines.append("")
        for license_file, content in license_files:
            text = content.decode("utf-8").replace("\r\n", "\n").replace("\r", "\n")
            lines.extend(
                [
                    "-" * 79,
                    f"Vendored file: {license_file['path']}",
                    f"SHA-256: {license_file['sha256']}",
                    "-" * 79,
                    text.rstrip("\n"),
                    "",
                ]
            )
    return "\n".join(lines).rstrip() + "\n"


def generate_notice(repo_root: Path, manifest_path: Path) -> str:
    repo_root = repo_root.resolve()
    manifest_path = manifest_path.resolve()
    manifest = read_manifest(manifest_path)
    validate_locked_crate(repo_root, manifest)
    validate_required_pins(repo_root, manifest)
    validate_locked_inputs(manifest)
    components = collect_licenses(manifest_path, manifest)
    return render_notice(manifest, components)


def main() -> int:
    args = parse_args()
    try:
        notice = generate_notice(args.repo_root, args.manifest)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(notice, encoding="utf-8", newline="\n")
    except NoticeError as exc:
        print(f"V8 notice generation failed: {exc}", file=sys.stderr)
        return 1
    print(f"Wrote {args.output} ({len(notice.encode('utf-8'))} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
