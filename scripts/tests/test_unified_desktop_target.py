"""Exercise the workflow's target directory with an actual independent workspace."""
from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


@unittest.skipUnless(shutil.which("cargo") and shutil.which("bash"), "Cargo and Bash required")
class UnifiedDesktopTargetTests(unittest.TestCase):
    def test_native_lanes_build_and_read_the_same_explicit_target(self):
        workflow = Path(".github/workflows/release-build.yml").read_text()
        native = workflow.split("  unified-native:", 1)[1].split("  assemble:", 1)[0]
        linux = native.split("export CARGO_TARGET_DIR=/work/target", 1)[1].split("      - name:", 1)[0]
        mac = native.split("      - name: Install frontend dependencies and build Desktop", 1)[1].split("      - name: Collect and package macOS installer", 1)[0]
        bundle = native.split("      - name: Collect and package macOS installer", 1)[1]
        self.assertIn('bundle_dir="$CARGO_TARGET_DIR/release/bundle/macos"', bundle)
        for lane, block, target in (("linux", linux, "export CARGO_TARGET_DIR=/work/target"),
                                    ("macos", mac, 'export CARGO_TARGET_DIR="$GITHUB_WORKSPACE/target"')):
            with self.subTest(lane=lane), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                desktop = root / "apps/desktop/src-tauri"
                (desktop / "src").mkdir(parents=True)
                (desktop / "fixture-tauri/src").mkdir(parents=True)
                (desktop / "Cargo.toml").write_text(
                    '[workspace]\n[package]\nname="webcodex-desktop"\nversion="0.1.0"\nedition="2021"\n'
                    '[dependencies]\ntauri={path="fixture-tauri"}\n'
                )
                (desktop / "src/main.rs").write_text('fn main() { println!("desktop fixture"); }\n')
                (desktop / "fixture-tauri/Cargo.toml").write_text(
                    '[package]\nname="tauri"\nversion="0.1.0"\nedition="2021"\n'
                    '[features]\ncustom-protocol=[]\n'
                )
                (desktop / "fixture-tauri/src/lib.rs").write_text("")
                env = {**os.environ, "GITHUB_WORKSPACE": str(root)}
                env.pop("CARGO_TARGET_DIR", None)
                subprocess.run(["cargo", "generate-lockfile", "--offline", "--manifest-path", str(desktop / "Cargo.toml")],
                               env=env, cwd=root, input=b"", capture_output=True, check=True, timeout=30)
                commands = [target.replace("/work/", str(root) + "/")]
                for prefix in ("cargo build ", "desktop=", 'test -x "$desktop"'):
                    commands.append(next(line.strip() for line in block.splitlines() if line.strip().startswith(prefix)))
                if lane == "macos":
                    self.assertLess(block.index(target), block.index("cargo build "))
                result = subprocess.run(["bash", "-euc", "\n".join(commands)], cwd=root, env=env,
                                        input=b"", capture_output=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr.decode())
                self.assertTrue((root / "target/release/webcodex-desktop").is_file())
                self.assertFalse((desktop / "target").exists())
