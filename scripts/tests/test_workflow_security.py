from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github" / "workflows"
EXTERNAL_ACTION_RE = re.compile(r"^\s*(?:-\s*)?uses:\s*([^\s]+)@([^\s#]+)", re.MULTILINE)
IMMUTABLE_SHA_RE = re.compile(r"^[0-9a-f]{40}$")


class WorkflowSecurityTests(unittest.TestCase):
    def workflow_files(self) -> list[Path]:
        files = sorted(WORKFLOWS.glob("*.yml")) + sorted(WORKFLOWS.glob("*.yaml"))
        self.assertTrue(files, "expected at least one GitHub Actions workflow")
        return files

    def test_every_workflow_defaults_to_contents_read(self) -> None:
        for path in self.workflow_files():
            text = path.read_text(encoding="utf-8")
            prefix, separator, _ = text.partition("\njobs:")
            self.assertTrue(separator, f"{path}: expected top-level jobs mapping")
            self.assertRegex(
                prefix,
                re.compile(r"(?m)^permissions:\s*$[\s\S]*?^\s{2}contents:\s*read\s*$"),
                f"{path}: top-level permissions must default contents to read",
            )

    def test_external_actions_are_pinned_to_full_commit_sha(self) -> None:
        violations: list[str] = []
        observed = 0
        for path in self.workflow_files():
            text = path.read_text(encoding="utf-8")
            for match in EXTERNAL_ACTION_RE.finditer(text):
                action, ref = match.groups()
                if action.startswith("./"):
                    continue
                observed += 1
                if not IMMUTABLE_SHA_RE.fullmatch(ref):
                    line = text.count("\n", 0, match.start()) + 1
                    violations.append(f"{path.relative_to(ROOT)}:{line}: {action}@{ref}")
        self.assertGreater(observed, 0, "expected external GitHub Actions references")
        self.assertFalse(
            violations,
            "external actions must use immutable 40-character commit SHAs:\n"
            + "\n".join(violations),
        )


if __name__ == "__main__":
    unittest.main()
