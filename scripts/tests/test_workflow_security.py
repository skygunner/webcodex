from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github" / "workflows"


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


if __name__ == "__main__":
    unittest.main()
