#!/usr/bin/env python3
"""Static safety checks for the Quiet for Codex release workflow."""

from __future__ import annotations

import json
import re
import tomllib
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "quiet-release.yml"
QUIET_CI_PATH = REPO_ROOT / ".github" / "workflows" / "quiet-ci.yml"
POWERSHELL_INSTALLER_PATH = REPO_ROOT / "scripts" / "release" / "install.ps1"
POWERSHELL_INSTALLER_TEST_PATH = (
    REPO_ROOT / "scripts" / "release" / "test_install_ps1.ps1"
)
SMOKE_PATH = REPO_ROOT / "scripts" / "release" / "smoke_quiet_package.py"
WORKFLOW_WATCHER_PATH = (
    REPO_ROOT / "scripts" / "release" / "watch_workflow_run.py"
)
SETUP_V8_ACTION_PATH = (
    REPO_ROOT / ".github" / "actions" / "setup-rusty-v8" / "action.yml"
)
SYMBOL_ARCHIVER_PATH = (
    REPO_ROOT / ".github" / "scripts" / "archive-release-symbols-and-strip-binaries.sh"
)
V8_MANIFEST_PATH = REPO_ROOT / "scripts" / "release" / "v8-notices-manifest.json"
TUI_UPDATE_ACTION_PATH = REPO_ROOT / "codex-rs" / "tui" / "src" / "update_action.rs"
TUI_UPDATES_PATH = REPO_ROOT / "codex-rs" / "tui" / "src" / "updates.rs"
TUI_UPDATE_PROMPT_PATH = REPO_ROOT / "codex-rs" / "tui" / "src" / "update_prompt.rs"
TUI_TOOLTIPS_PATH = REPO_ROOT / "codex-rs" / "tui" / "src" / "tooltips.rs"
README_PATH = REPO_ROOT / "README.md"
INSTALL_DOC_PATH = REPO_ROOT / "docs" / "install.md"
SUPPORT_PATH = REPO_ROOT / "SUPPORT.md"
CHANGELOG_PATH = REPO_ROOT / "CHANGELOG.md"
FORK_CHANGES_PATH = REPO_ROOT / "FORK_CHANGES.md"
BUG_REPORT_PATH = REPO_ROOT / ".github" / "ISSUE_TEMPLATE" / "bug-report.yml"
JUSTFILE_PATH = REPO_ROOT / "justfile"
TUI_MANIFEST_PATH = REPO_ROOT / "codex-rs" / "tui" / "Cargo.toml"
INSTALLER_PATHS = (
    REPO_ROOT / "scripts" / "release" / "install.sh",
    POWERSHELL_INSTALLER_PATH,
)
EXPECTED_TARGETS = {
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
}
EXPECTED_V8_TARGETS = EXPECTED_TARGETS | {"x86_64-unknown-linux-gnu"}
LITERAL_TARGET_RE = re.compile(r"^\s+target: ([a-z0-9_][a-z0-9_.-]+)$", re.MULTILINE)


class ReleaseWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW_PATH.read_text(encoding="utf-8")
        cls.quiet_ci = QUIET_CI_PATH.read_text(encoding="utf-8")
        cls.powershell_installer = POWERSHELL_INSTALLER_PATH.read_text(encoding="utf-8")
        cls.powershell_installer_test = POWERSHELL_INSTALLER_TEST_PATH.read_text(
            encoding="utf-8"
        )
        cls.smoke = SMOKE_PATH.read_text(encoding="utf-8")
        cls.workflow_watcher = WORKFLOW_WATCHER_PATH.read_text(encoding="utf-8")
        cls.setup_v8_action = SETUP_V8_ACTION_PATH.read_text(encoding="utf-8")
        cls.symbol_archiver = SYMBOL_ARCHIVER_PATH.read_text(encoding="utf-8")
        cls.v8_manifest = json.loads(V8_MANIFEST_PATH.read_text(encoding="utf-8"))
        cls.tui_update_action = TUI_UPDATE_ACTION_PATH.read_text(encoding="utf-8")
        cls.tui_updates = TUI_UPDATES_PATH.read_text(encoding="utf-8")
        cls.tui_update_prompt = TUI_UPDATE_PROMPT_PATH.read_text(encoding="utf-8")
        cls.tui_tooltips = TUI_TOOLTIPS_PATH.read_text(encoding="utf-8")
        cls.readme = README_PATH.read_text(encoding="utf-8")
        cls.install_doc = INSTALL_DOC_PATH.read_text(encoding="utf-8")
        cls.support = SUPPORT_PATH.read_text(encoding="utf-8")
        cls.fork_changes = FORK_CHANGES_PATH.read_text(encoding="utf-8")
        cls.bug_report = BUG_REPORT_PATH.read_text(encoding="utf-8")
        cls.justfile = JUSTFILE_PATH.read_text(encoding="utf-8")
        cls.tui_manifest = tomllib.loads(TUI_MANIFEST_PATH.read_text(encoding="utf-8"))
        changelog = CHANGELOG_PATH.read_text(encoding="utf-8")
        # Mirror the workflow: the release version is the FIRST released
        # heading, whichever channel it is. Searching for the first *beta*
        # heading instead meant that once a stable entry was added on top, this
        # kept validating the previous beta's install pins while a stable
        # release was being cut, so the pin checks below silently tested the
        # wrong version.
        release_heading = re.search(
            r"^## ([0-9]+\.[0-9]+\.[0-9]+(?:-beta\.[1-9][0-9]*)?) - ",
            changelog,
            re.MULTILINE,
        )
        if release_heading is None:
            raise AssertionError("CHANGELOG.md has no canonical Quiet release heading")
        cls.release_version = release_heading.group(1)

    def test_build_matrix_covers_exactly_six_supported_targets(self) -> None:
        matrix = self.workflow.split("matrix:", 1)[1].split("steps:", 1)[0]
        targets = set(re.findall(r"^\s+target: ([^\s]+)$", matrix, re.MULTILINE))
        self.assertEqual(targets, EXPECTED_TARGETS)

    def test_published_release_assets_cannot_be_replaced(self) -> None:
        for forbidden in ("--clobber", "gh release upload", "gh release edit"):
            self.assertNotIn(forbidden, self.workflow)

        existing_release_gate = self.workflow.rindex('gh release view "$TAG"')
        refusal = self.workflow.rindex("Published assets are immutable.")
        create = self.workflow.index('gh release create "$TAG"')
        self.assertLess(existing_release_gate, refusal)
        self.assertLess(refusal, create)
        self.assertIn("exit 1", self.workflow[refusal:create])
        self.assertEqual(self.workflow.count('gh release view "$TAG"'), 2)

    def test_release_is_created_once_after_asset_validation(self) -> None:
        self.assertEqual(self.workflow.count('gh release create "$TAG"'), 1)
        self.assertIn(
            "Release archive set does not match the six packages and six symbol sidecars",
            self.workflow,
        )
        self.assertIn("sha256sum --check SHA256SUMS", self.workflow)
        self.assertIn("--verify-tag", self.workflow)
        self.assertIn("--prerelease", self.workflow)
        self.assertIn("--latest=false", self.workflow)

    def test_release_builds_and_publishes_one_peeled_tag_commit(self) -> None:
        self.assertIn("commit: ${{ steps.tag.outputs.commit }}", self.workflow)
        self.assertIn('echo "commit=$commit"', self.workflow)
        self.assertIn('} >> "$GITHUB_OUTPUT"', self.workflow)
        self.assertGreaterEqual(self.workflow.count("git ls-remote --exit-code"), 2)
        # validate, verify, build, and publish. Publish joined this set when the
        # release notes started being read from CHANGELOG.md; every job in the
        # release path checks out the one validated commit and nothing else.
        self.assertEqual(
            self.workflow.count("ref: ${{ needs.validate.outputs.commit }}"), 4
        )
        self.assertNotIn("ref: ${{ needs.validate.outputs.tag }}", self.workflow)
        self.assertIn(
            "EXPECTED_COMMIT: ${{ needs.validate.outputs.commit }}", self.workflow
        )
        self.assertIn('remote_commit" != "$EXPECTED_COMMIT', self.workflow)
        self.assertIn("Source commit: %s", self.workflow)

    def test_release_identity_is_canonical_and_matches_public_main(self) -> None:
        self.assertIn("-beta\\.[1-9][0-9]*", self.workflow)
        self.assertIn("changelog_version", self.workflow)
        self.assertIn("does not match first released CHANGELOG version", self.workflow)
        self.assertEqual(self.workflow.count('main_ref="refs/heads/main"'), 2)
        self.assertIn(
            "Release tag must identify the current public main tip", self.workflow
        )
        self.assertIn("Public main moved after validation", self.workflow)

    def test_release_preserves_documented_upstream_base_ancestry(self) -> None:
        validate_job = self.workflow.split("  validate:\n", 1)[1].split(
            "  verify:\n", 1
        )[0]
        self.assertIn("fetch-depth: 0", validate_job)
        self.assertIn(
            'git merge-base --is-ancestor "$upstream_base" HEAD', validate_job
        )
        self.assertIn(
            '"$GITHUB_REF" != "refs/tags/$tag"',
            validate_job,
        )
        self.assertIn(
            "This keeps build provenance bound to the packaged tag.",
            validate_job,
        )
        self.assertIn(
            "sed -nE 's/^- Base release:",
            validate_job,
        )
        self.assertIn("sed -nE 's/^- Base commit:", validate_job)
        self.assertIn(
            'upstream_remote="https://github.com/openai/codex.git"', validate_job
        )
        self.assertIn(
            'upstream_refs="$(git ls-remote --exit-code "$upstream_remote"',
            validate_job,
        )
        self.assertIn(
            'direct_upstream_ref="refs/tags/$documented_release"', validate_job
        )
        self.assertIn(
            'peeled_upstream_ref="refs/tags/$documented_release^{}"', validate_job
        )
        self.assertIn(
            'documented_commit="${peeled_upstream_commit:-$direct_upstream_commit}"',
            validate_job,
        )
        self.assertNotIn('git rev-parse "$documented_release^{commit}"', validate_job)
        self.assertIn('documented_commit" != "$upstream_base', validate_job)
        self.assertNotRegex(validate_job, r'upstream_base="[0-9a-f]{40}"')
        self.assertIn("breaks fork provenance", validate_job)

    def test_release_attests_and_verifies_every_archive(self) -> None:
        action = "actions/attest@f7c74d28b9d84cb8768d0b8ca14a4bac6ef463e6"
        self.assertEqual(self.workflow.count(action), 2)
        self.assertEqual(self.workflow.count("subject-path: dist/codex-quiet-*"), 1)
        self.assertEqual(self.workflow.count("subject-path: dist/codex-*"), 1)

        canary = self.workflow.split("  attestation-canary:\n", 1)[1].split(
            "  validate:\n", 1
        )[0]
        self.assertIn("inputs.mode == 'attestation-canary'", canary)
        self.assertIn("contents: read", canary)
        self.assertIn("id-token: write", canary)
        self.assertIn("attestations: write", canary)
        self.assertIn("--source-digest", canary)
        self.assertIn("--source-ref", canary)
        self.assertIn("--deny-self-hosted-runners", canary)
        self.assertNotIn("contents: write", canary)

        build = self.workflow.split("  build:\n", 1)[1].split("  publish:\n", 1)[0]
        self.assertIn("contents: read", build)
        self.assertIn("id-token: write", build)
        self.assertIn("attestations: write", build)
        self.assertLess(
            build.index("Finalize, launch-test, and archive"),
            build.index("Generate build provenance"),
        )
        self.assertLess(
            build.index("Generate build provenance"),
            build.index("actions/upload-artifact@"),
        )

        publish = self.workflow.split("  publish:\n", 1)[1]
        self.assertIn("attestations: read", publish)
        self.assertIn('gh attestation verify "dist/$archive"', publish)
        self.assertIn("--signer-workflow", publish)
        self.assertIn('--source-digest "$COMMIT"', publish)
        self.assertIn('--source-ref "$GITHUB_REF"', publish)
        self.assertIn("--deny-self-hosted-runners", publish)
        self.assertEqual(self.workflow.count("for attempt in 1 2 3; do"), 2)
        self.assertIn(
            "Build provenance did not verify for $archive after three attempts.",
            publish,
        )

    def test_published_release_is_verified_immutable_without_admin_api(self) -> None:
        self.assertNotIn('"repos/$GITHUB_REPOSITORY/immutable-releases"', self.workflow)
        endpoint = '"repos/$GITHUB_REPOSITORY/releases/tags/$TAG"'
        self.assertEqual(self.workflow.count(endpoint), 1)
        self.assertEqual(self.workflow.count("--jq '.immutable'"), 1)
        create = self.workflow.index('gh release create "$TAG"')
        verify = self.workflow.index(endpoint)
        self.assertLess(create, verify)
        self.assertIn("Published release is not immutable", self.workflow)

    def test_public_release_identity(self) -> None:
        self.assertIn("Quiet for Codex ${TAG#quiet-v}", self.workflow)
        for installer_path in INSTALLER_PATHS:
            installer = installer_path.read_text(encoding="utf-8")
            self.assertIn("maherr/quiet-for-codex", installer)
            self.assertNotIn("maherr/codex-quiet", installer)

    def test_v8_notices_are_generated_and_required_by_finalization(self) -> None:
        self.assertIn("generate_v8_notices.py", self.workflow)
        self.assertIn("license-dist/V8_RUSTY_V8_NOTICES.txt", self.workflow)
        self.assertIn(
            '--v8-notices "$RUNNER_TEMP/quiet-licenses/V8_RUSTY_V8_NOTICES.txt"',
            self.workflow,
        )
        self.assertIn("quiet-release-licenses", self.workflow)
        self.assertNotIn("quiet-rust-licenses", self.workflow)

    def test_every_platform_uses_manifest_pinned_v8_archive_and_binding(self) -> None:
        setup_action = "uses: ./.github/actions/setup-rusty-v8"
        self.assertEqual(self.workflow.count(setup_action), 2)
        self.assertEqual(self.quiet_ci.count(setup_action), 4)
        self.assertIn("prepare_v8_artifacts.py", self.setup_v8_action)
        self.assertIn('--github-env "${GITHUB_ENV}"', self.setup_v8_action)
        self.assertNotIn("rusty_v8_release_${TARGET}.sha256", self.setup_v8_action)

        artifacts = self.v8_manifest["lockedInputs"]["artifacts"]
        configured_targets = set(LITERAL_TARGET_RE.findall(self.workflow)) | set(
            LITERAL_TARGET_RE.findall(self.quiet_ci)
        )
        self.assertEqual(configured_targets, EXPECTED_V8_TARGETS)
        self.assertEqual(
            {artifact["target"] for artifact in artifacts}, configured_targets
        )
        for artifact in artifacts:
            self.assertRegex(artifact["sha256"], r"^[0-9a-f]{64}$")
            self.assertRegex(artifact["bindingSha256"], r"^[0-9a-f]{64}$")
            binding_sources = {
                key for key in ("bindingUrl", "bindingCratePath") if key in artifact
            }
            self.assertEqual(len(binding_sources), 1)

        windows = [
            artifact
            for artifact in artifacts
            if artifact["target"].endswith("-pc-windows-msvc")
        ]
        self.assertEqual(len(windows), 2)
        # Upstream rust-v0.147.0 builds Windows rusty_v8 artifacts in its own
        # release (ptrcomp_sandbox_release) with standalone binding files, so the
        # Windows pins use bindingUrl like the unix targets.
        self.assertTrue(all("bindingUrl" in artifact for artifact in windows))

    def test_release_profile_disables_stock_updates_and_announcements(self) -> None:
        self.assertIn("--cargo-profile release", self.workflow)
        self.assertIn(
            'CARGO_PROFILE_RELEASE_DEBUG: "line-tables-only"', self.workflow
        )
        self.assertNotIn('CARGO_PROFILE_RELEASE_DEBUG: "none"', self.workflow)
        self.assertRegex(
            self.tui_update_action,
            r"(?s)#\[cfg\(not\(debug_assertions\)\)\]\s+"
            r"pub fn get_update_action\(\) -> Option<UpdateAction> \{.*?\n\s*None\n\}",
        )
        self.assertGreaterEqual(
            self.tui_updates.count('CODEX_CLI_DISPLAY_NAME == "codex-quiet"'), 2
        )
        self.assertIn(
            "https://api.github.com/repos/maherr/quiet-for-codex/releases?per_page=20",
            self.tui_updates,
        )
        self.assertIn(
            'const RELEASE_NOTES_URL: &str = "https://github.com/maherr/quiet-for-codex/releases";',
            self.tui_update_prompt,
        )
        self.assertIn('CODEX_CLI_DISPLAY_NAME != "codex-quiet"', self.tui_tooltips)

    def test_every_release_has_a_build_id_keyed_symbol_sidecar(self) -> None:
        self.assertIn(
            "archive-release-symbols-and-strip-binaries.sh", self.workflow
        )
        self.assertIn('--binaries "codex codex-code-mode-host"', self.workflow)
        self.assertEqual(self.workflow.count('"codex-symbols-$VERSION-'), 6)
        self.assertEqual(self.workflow.count('"dist/codex-symbols-$VERSION-'), 6)
        self.assertIn("BUILD_IDS.txt", self.symbol_archiver)
        self.assertIn("readelf -n", self.symbol_archiver)
        self.assertIn("dwarfdump --uuid", self.symbol_archiver)
        self.assertIn("dsymutil", self.symbol_archiver)
        self.assertIn(".pdb", self.symbol_archiver)
        self.assertIn("shipped_binary_sha256", self.symbol_archiver)

    def test_packaged_smoke_replays_real_paginated_history(self) -> None:
        for required in (
            '"history_mode": "paginated"',
            '"type": "AgentMessage"',
            '"phase": "final_answer"',
            '"type": "Reasoning"',
            '"method": "thread/resume"',
            '"sandbox": "read-only"',
            '"--no-alt-screen"',
            "RESTORING_HISTORY_MARKER",
            "SMOKE_FINAL_MARKER",
        ):
            self.assertIn(required, self.smoke)
        self.assertNotIn("Replay smoke skipped", self.smoke)

    def test_release_watcher_polls_api_to_a_dynamic_conclusion(self) -> None:
        self.assertIn('CANONICAL_REPOSITORY = "maherr/quiet-for-codex"', self.workflow_watcher)
        self.assertIn('["gh", "api", endpoint]', self.workflow_watcher)
        self.assertNotIn("gh run watch", self.workflow_watcher)
        self.assertIn('state[0] == "completed"', self.workflow_watcher)
        self.assertIn('state[1] != "success"', self.workflow_watcher)
        self.assertIn('run.get("head_sha")', self.workflow_watcher)
        self.assertIn('run.get("head_branch")', self.workflow_watcher)
        self.assertIn('parser.add_argument("--sha", required=True', self.workflow_watcher)
        self.assertIn('parser.add_argument("--ref", required=True', self.workflow_watcher)
        self.assertIn("fetch_all_jobs", self.workflow_watcher)
        self.assertIn("page={page}", self.workflow_watcher)
        self.assertIn("no dynamically discovered Build matrix jobs", self.workflow_watcher)
        self.assertIn("conclusion == \"skipped\"", self.workflow_watcher)
        self.assertIn("ALLOWED_SKIPPED_JOBS", self.workflow_watcher)

    def test_all_daemon_managed_routes_are_black_box_denied(self) -> None:
        routes = (
            ("app-server", "daemon", "bootstrap"),
            ("app-server", "daemon", "start"),
            ("app-server", "daemon", "stop"),
            ("app-server", "daemon", "restart"),
            ("app-server", "daemon", "enable-remote-control"),
            ("app-server", "daemon", "disable-remote-control"),
            ("app-server", "daemon", "pid-update-loop"),
            ("app-server", "daemon", "version"),
            ("remote-control",),
            ("remote-control", "start"),
            ("remote-control", "stop"),
            ("remote-control", "pair"),
        )
        for route in routes:
            route_items = ", ".join(f'"{part}"' for part in route)
            if len(route) == 1:
                route_items += ","
            smoke_route = f"({route_items}),"
            ci_route = f'"{" ".join(route)}"'
            self.assertIn(smoke_route, self.smoke)
            self.assertIn(ci_route, self.quiet_ci)
            self.assertIn(ci_route, self.workflow)

        disabled_message = (
            "daemon-managed app-server routes are disabled in Quiet for Codex because "
            "the upstream implementation installs and updates stock Codex"
        )
        self.assertIn(disabled_message, self.smoke.replace('"\n    "', ""))
        self.assertIn(disabled_message, self.quiet_ci)
        self.assertIn("result.returncode == 0", self.smoke)
        self.assertIn("QUIET_DAEMON_DISABLED_MESSAGE not in output", self.smoke)

    def test_archive_smoke_executes_every_bundled_tool_family(self) -> None:
        for expected_probe in (
            '[str(ripgrep), "--version"]',
            '[str(bwrap), "--version"]',
            '"codex-windows-sandbox-setup.exe"',
            '"expected payload argument"',
            '"codex-command-runner.exe"',
            '"no pipe-in provided"',
        ):
            self.assertIn(expected_probe, self.smoke)

    def test_release_reverifies_the_exact_tagged_commit(self) -> None:
        verify_job = self.workflow.split("  verify:\n", 1)[1].split("  licenses:\n", 1)[
            0
        ]
        self.assertIn("ref: ${{ needs.validate.outputs.commit }}", verify_job)
        for required_check in (
            "scripts/ci/check_embedded_skill_identity.py",
            "scripts/codex_package/test_ripgrep.py",
            "test_generate_v8_notices.py",
            "test_prepare_v8_artifacts.py",
            "test_archive_release_symbols.py",
            "test_finalize_quiet_package.py",
            "test_smoke_quiet_package.py",
            "test_watch_workflow_run.py",
            "test_install_sh.py",
            "test_release_workflow.py",
            "codex-tui --lib",
            "quiet_build_does_not_register_desktop_app_subcommand",
            "release_list_includes_prerelease_channel",
            "-p codex-cli --test update",
            "session_new_falls_back_when_zsh_fork_enabled_without_packaged_zsh",
            "streamlined_success_page_does_not_handoff_to_the_stock_desktop_app",
            "login_account_chatgpt_ignores_stock_desktop_success_page_request",
            "quiet_feedback_uploads_are_disabled",
            "quiet_feedback_upload_api_stays_disabled",
            "feedback_command_reports_uploads_disabled",
            "quiet_commands_are_hidden_from_command_popup",
            "exec_summary_uses_quiet_identity_and_exact_build_version",
        ):
            self.assertIn(required_check, verify_job)

        identity_test = verify_job.index(
            "exec_summary_uses_quiet_identity_and_exact_build_version"
        )
        identity_command = verify_job[max(0, identity_test - 500) : identity_test]
        self.assertIn(
            'CODEX_QUIET_VERSION="${{ needs.validate.outputs.version }}"',
            identity_command,
        )
        self.assertIn(
            'CODEX_QUIET_DISPLAY_VERSION="codex-quiet ${{ needs.validate.outputs.version }}'
            ' (codex ${{ needs.validate.outputs.codex_base }})"',
            identity_command,
        )
        self.assertNotIn("\n    env:\n      CODEX_QUIET_VERSION:", verify_job)
        # Verification gates PUBLISH, not build. Build starts as soon as the tag
        # validates so the six platform compiles overlap the verification
        # compile; publish still cannot run until verify has passed, so nothing
        # unverified can reach a released asset.
        build_needs = self.workflow.split("  build:\n", 1)[1].split("    runs-on:", 1)[
            0
        ]
        self.assertNotIn("- verify", build_needs)
        publish_needs = self.workflow.split("  publish:\n", 1)[1].split(
            "    runs-on:", 1
        )[0]
        self.assertIn("- verify", publish_needs)
        self.assertIn("- build", publish_needs)

    def test_publish_checks_out_the_source_it_reads(self) -> None:
        """Publish reads CHANGELOG.md, so it needs the tree, in the right order.

        The notes used to be a hardcoded string, so the job needed no source and
        had no checkout. Generating them from CHANGELOG.md made that a
        FileNotFoundError, and only at the publish step, after six platform
        builds had already succeeded and the tag was immutable.

        Order is load-bearing: actions/checkout defaults to clean, so a checkout
        placed after download-artifact deletes the archives being published.
        """
        body = self.workflow.split("\n  publish:\n", 1)[1]
        # Cut at the next top-level job header so this reads only publish.
        next_job = re.search(r"\n  [a-z][a-z0-9-]*:\n", body)
        publish = body[: next_job.start()] if next_job else body

        self.assertIn("actions/checkout@", publish)
        self.assertIn("ref: ${{ needs.validate.outputs.commit }}", publish)
        self.assertIn("persist-credentials: false", publish)
        self.assertLess(
            publish.index("actions/checkout@"),
            publish.index("actions/download-artifact@"),
            "checkout must precede download-artifact or it cleans away the archives",
        )
        self.assertIn("CHANGELOG.md", publish)

    def test_tag_tracks_quiet_version_and_cargo_tracks_codex_base(self) -> None:
        """The two version axes must stay separate and independently gated.

        The tag carries Quiet's own version and must equal QUIET_VERSION. The
        Cargo workspace version carries the upstream Codex base and must be what
        FORK_CHANGES.md documents. Collapsing these was what made a second Quiet
        release on an unchanged Codex base unexpressible.
        """
        quiet_version = (
            (REPO_ROOT / "QUIET_VERSION").read_text(encoding="utf-8").strip()
        )
        self.assertRegex(
            quiet_version, r"^[0-9]+\.[0-9]+\.[0-9]+(-beta\.[1-9][0-9]*)?$"
        )
        self.assertEqual(quiet_version, self.release_version)

        self.assertIn(
            "quiet_version=\"$(tr -d '[:space:]' < QUIET_VERSION)\"", self.workflow
        )
        self.assertIn("does not match QUIET_VERSION", self.workflow)
        self.assertIn(
            'if [[ "$documented_release" != "rust-v$codex_base" ]]', self.workflow
        )
        # The tag base must no longer be compared against the Cargo version.
        self.assertNotIn("does not match Cargo version", self.workflow)
        self.assertIn("codex_base: ${{ steps.tag.outputs.codex_base }}", self.workflow)

    def test_full_tui_suite_runs_once_and_serially(self) -> None:
        command = (
            "cargo test --locked --target x86_64-unknown-linux-gnu "
            "-p codex-tui --lib -- --test-threads=1"
        )
        for workflow in (self.quiet_ci, self.workflow):
            tui_step = workflow.split("      - name: Run TUI library tests\n", 1)[
                1
            ].split("      - name:", 1)[0]
            self.assertEqual(tui_step.count(command), 1)
            self.assertNotIn("--skip", tui_step)

    def test_quiet_ci_runs_targeted_fork_safety_tests(self) -> None:
        for test_name in (
            "scripts/ci/check_embedded_skill_identity.py",
            "scripts/codex_package/test_ripgrep.py",
            "quiet_build_does_not_register_desktop_app_subcommand",
            "release_list_includes_prerelease_channel",
            "-p codex-cli --test update",
            "session_new_falls_back_when_zsh_fork_enabled_without_packaged_zsh",
            "streamlined_success_page_does_not_handoff_to_the_stock_desktop_app",
            "login_account_chatgpt_ignores_stock_desktop_success_page_request",
            "quiet_feedback_uploads_are_disabled",
            "quiet_feedback_upload_api_stays_disabled",
            "feedback_command_reports_uploads_disabled",
            "quiet_commands_are_hidden_from_command_popup",
            "exec_summary_uses_quiet_identity_and_exact_build_version",
            "default_command_popup_items_snapshot",
        ):
            self.assertIn(test_name, self.quiet_ci)

    def test_snapshot_refresh_is_read_only_and_source_bound(self) -> None:
        refresh = self.quiet_ci.split("  snapshot-refresh:\n", 1)[1].split(
            "  quality:\n", 1
        )[0]
        self.assertIn("inputs.refresh_snapshots", refresh)
        self.assertIn("contents: read", refresh)
        self.assertNotIn("contents: write", refresh)
        self.assertIn("INSTA_UPDATE=always cargo test --locked", refresh)
        self.assertIn("codex-rs/*/snapshots/*.snap", refresh)
        self.assertIn("git diff --binary -- codex-rs", refresh)
        self.assertIn("snapshot-refresh/source-sha.txt", refresh)
        self.assertIn("quiet-snapshot-refresh-${{ github.sha }}", refresh)
        self.assertIn("retention-days: 7", refresh)
        self.assertIn("git diff --name-only --no-renames -z", refresh)
        self.assertIn("git ls-files --others --exclude-standard -z", refresh)
        self.assertIn('git add -N -- "${untracked[@]}"', refresh)
        self.assertNotIn("git status --porcelain", refresh)
        self.assertIn(
            "github.event_name == 'workflow_dispatch' && inputs.refresh_snapshots "
            "&& 'snapshot-refresh' || 'checks'",
            self.quiet_ci,
        )
        self.assertEqual(
            self.quiet_ci.count(
                "github.event_name != 'workflow_dispatch' || !inputs.refresh_snapshots"
            ),
            2,
        )

    def test_quiet_ci_builds_source_bound_hosted_linux_candidate(self) -> None:
        candidate = self.quiet_ci.split("  linux-candidate:\n", 1)[1].split(
            "  platform-check:\n", 1
        )[0]
        for required in (
            "github.event_name == 'push'",
            "refs/heads/tui/quiet-",
            "x86_64-unknown-linux-musl",
            'CARGO_INCREMENTAL: "0"',
            'CARGO_PROFILE_RELEASE_DEBUG: "line-tables-only"',
            "CODEX_QUIET_DISPLAY_VERSION=codex-quiet",
            "scripts/build_codex_package.py",
            "--cargo-profile release",
            "--zsh-manifest scripts/release/no-zsh.json",
            "codex-quiet-candidate-$GITHUB_SHA-$TARGET.tar.gz",
            'actual="$("$package_dir/bin/codex" --version)"',
            "candidate-dist/source-sha.txt",
            "sha256sum",
            "quiet-linux-candidate-${{ github.sha }}",
            "retention-days: 7",
        ):
            self.assertIn(required, candidate)
        self.assertNotIn("contents: write", candidate)
        self.assertNotIn("session", candidate.lower())

    def test_windows_installer_is_powershell_51_safe_and_relocatable(self) -> None:
        self.assertEqual(self.powershell_installer.count("-UseBasicParsing"), 3)
        self.assertIn(
            '$ShimTarget = "%~dp0..\\releases\\$Version-$Target\\bin\\codex-quiet.exe"',
            self.powershell_installer,
        )
        self.assertNotIn("$InstalledExe", self.powershell_installer)
        self.assertIn('"current.txt") -Encoding utf8', self.powershell_installer)
        self.assertIn("$([char]0x4F8B)", self.powershell_installer_test)
        self.assertIn("$Shim --version", self.powershell_installer_test)
        self.assertIn("$Shim --help", self.powershell_installer_test)
        self.assertIn("SHIM_ARGUMENT_OK", self.powershell_installer_test)
        self.assertIn(
            "function global:Invoke-RestMethod", self.powershell_installer_test
        )
        self.assertIn(
            'tag_name = "quiet-v$global:QuietInstallerTestVersion"',
            self.powershell_installer_test,
        )
        self.assertIn("Windows PowerShell 5.1", self.quiet_ci)
        self.assertIn("powershell.exe", self.quiet_ci)

    def test_windows_bootstrap_uses_child_scope_without_iex(self) -> None:
        for documentation in (self.readme, self.install_doc):
            self.assertIn(
                "& ([scriptblock]::Create((irm -UseBasicParsing ", documentation
            )
            self.assertNotIn("| iex", documentation.lower())
            self.assertIn(
                "raw.githubusercontent.com/maherr/quiet-for-codex/"
                f"quiet-v{self.release_version}/scripts/release/install.sh",
                documentation,
            )
            self.assertIn(
                "raw.githubusercontent.com/maherr/quiet-for-codex/"
                f"quiet-v{self.release_version}/scripts/release/install.ps1",
                documentation,
            )
            self.assertNotIn(
                "raw.githubusercontent.com/maherr/quiet-for-codex/main/",
                documentation,
            )
        self.assertIn(
            f"placeholder: codex-quiet {self.release_version}", self.bug_report
        )

    def test_readme_has_compact_before_after_comparison(self) -> None:
        for text in (
            "Before: Official Codex (8 blocks, 16 timeline lines)",
            "After settlement: Quiet for Codex (1 row, 1 timeline line)",
            "That is 94% less vertical timeline",
            "completed row stable",
            "clicked, hovered, visible, or latest compact group",
            "Alt+O",
            "Ctrl+T",
        ):
            self.assertIn(text, self.readme)
        self.assertNotIn("official-vs-quiet.png", self.readme)

    def test_support_uses_stable_product_framing_with_target_specific_beta_tiers(
        self,
    ) -> None:
        self.assertIn(
            "Quiet for Codex is an unofficial, community-maintained release",
            self.support,
        )
        self.assertNotIn("Quiet for Codex is a public beta", self.support)
        self.assertIn("Tested beta", self.support)
        self.assertIn("CI beta", self.support)
        self.assertIn("CI preview", self.support)

    def test_quiet_benchmark_isolated_from_normal_tui_targets(self) -> None:
        self.assertFalse(self.tui_manifest["package"]["autobenches"])
        self.assertEqual(self.tui_manifest["features"]["quiet-bench"], ["dep:divan"])
        self.assertTrue(self.tui_manifest["dependencies"]["divan"]["optional"])

        bins = {target["name"]: target for target in self.tui_manifest["bin"]}
        self.assertFalse(bins["codex-tui"]["bench"])
        self.assertFalse(bins["md-events"]["bench"])
        self.assertEqual(
            bins["quiet-render-bench"]["required-features"], ["quiet-bench"]
        )
        self.assertFalse(bins["quiet-render-bench"]["test"])
        self.assertFalse(bins["quiet-render-bench"]["bench"])
        self.assertNotIn("bench", self.tui_manifest)

        self.assertIn(
            "cargo run --release --jobs 1 -p codex-tui --bin quiet-render-bench "
            "--features quiet-bench {{ args }}",
            self.justfile,
        )
        self.assertIn("just quiet-bench {{ args }}", self.justfile)
        self.assertNotIn("--features codex-tui/quiet-bench", self.justfile)

    def test_manual_install_docs_verify_checksums_on_every_platform(self) -> None:
        for required in (
            "expected=$(awk",
            "command -v sha256sum",
            "shasum -a 256",
            "Get-FileHash -Algorithm SHA256",
            "gh attestation verify",
            "--signer-workflow",
            "--deny-self-hosted-runners",
            "Together, the checksum command for your platform",
            "A checksum is not a publisher signature",
            "unsigned binary notes",
        ):
            self.assertIn(required, self.install_doc)


if __name__ == "__main__":
    unittest.main()
