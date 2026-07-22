#!/usr/bin/env python3
"""Static safety checks for the Quiet for Codex release workflow."""

from __future__ import annotations

import json
import re
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
SETUP_V8_ACTION_PATH = (
    REPO_ROOT / ".github" / "actions" / "setup-rusty-v8" / "action.yml"
)
V8_MANIFEST_PATH = REPO_ROOT / "scripts" / "release" / "v8-notices-manifest.json"
TUI_UPDATE_ACTION_PATH = REPO_ROOT / "codex-rs" / "tui" / "src" / "update_action.rs"
TUI_UPDATES_PATH = REPO_ROOT / "codex-rs" / "tui" / "src" / "updates.rs"
TUI_UPDATE_PROMPT_PATH = REPO_ROOT / "codex-rs" / "tui" / "src" / "update_prompt.rs"
TUI_TOOLTIPS_PATH = REPO_ROOT / "codex-rs" / "tui" / "src" / "tooltips.rs"
README_PATH = REPO_ROOT / "README.md"
INSTALL_DOC_PATH = REPO_ROOT / "docs" / "install.md"
CHANGELOG_PATH = REPO_ROOT / "CHANGELOG.md"
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
        cls.setup_v8_action = SETUP_V8_ACTION_PATH.read_text(encoding="utf-8")
        cls.v8_manifest = json.loads(V8_MANIFEST_PATH.read_text(encoding="utf-8"))
        cls.tui_update_action = TUI_UPDATE_ACTION_PATH.read_text(encoding="utf-8")
        cls.tui_updates = TUI_UPDATES_PATH.read_text(encoding="utf-8")
        cls.tui_update_prompt = TUI_UPDATE_PROMPT_PATH.read_text(encoding="utf-8")
        cls.tui_tooltips = TUI_TOOLTIPS_PATH.read_text(encoding="utf-8")
        cls.readme = README_PATH.read_text(encoding="utf-8")
        cls.install_doc = INSTALL_DOC_PATH.read_text(encoding="utf-8")
        changelog = CHANGELOG_PATH.read_text(encoding="utf-8")
        release_heading = re.search(
            r"^## ([0-9]+\.[0-9]+\.[0-9]+-beta\.[1-9][0-9]*) - ",
            changelog,
            re.MULTILINE,
        )
        if release_heading is None:
            raise AssertionError("CHANGELOG.md has no canonical Quiet beta heading")
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
            "Release archive set does not match the six expected targets", self.workflow
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
        self.assertEqual(
            self.workflow.count("ref: ${{ needs.validate.outputs.commit }}"), 3
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
            'upstream_base="25af12f7e61572b0bc18ddb1008be543b91519b0"',
            validate_job,
        )
        self.assertIn("breaks fork provenance", validate_job)

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
        self.assertEqual(self.quiet_ci.count(setup_action), 2)
        self.assertIn("prepare_v8_artifacts.py", self.setup_v8_action)
        self.assertIn('--github-env "${GITHUB_ENV}"', self.setup_v8_action)
        self.assertNotIn("rusty_v8_release_${TARGET}.sha256", self.setup_v8_action)

        artifacts = self.v8_manifest["lockedInputs"]["artifacts"]
        self.assertEqual(
            {artifact["target"] for artifact in artifacts}, EXPECTED_TARGETS
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
        self.assertTrue(all("bindingCratePath" in artifact for artifact in windows))

    def test_release_profile_disables_stock_updates_and_announcements(self) -> None:
        self.assertIn("--cargo-profile release", self.workflow)
        self.assertIn('CARGO_PROFILE_RELEASE_DEBUG: "none"', self.workflow)
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
            "test_finalize_quiet_package.py",
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
            'CODEX_QUIET_DISPLAY_VERSION="codex-quiet ${{ needs.validate.outputs.version }}"',
            identity_command,
        )
        self.assertNotIn("\n    env:\n      CODEX_QUIET_VERSION:", verify_job)
        build_needs = self.workflow.split("  build:\n", 1)[1].split("    runs-on:", 1)[
            0
        ]
        self.assertIn("- verify", build_needs)

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

    def test_manual_install_docs_verify_checksums_on_every_platform(self) -> None:
        for required in (
            "expected=$(awk",
            "command -v sha256sum",
            "shasum -a 256",
            "Get-FileHash -Algorithm SHA256",
            "A checksum is not a publisher signature",
            "unsigned binary notes",
        ):
            self.assertIn(required, self.install_doc)


if __name__ == "__main__":
    unittest.main()
