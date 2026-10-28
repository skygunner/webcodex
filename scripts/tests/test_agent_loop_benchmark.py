from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

from scripts import agent_loop_benchmark as benchmark
from scripts import agent_loop_report as report


class AgentLoopBenchmarkTests(unittest.TestCase):
    def test_long_handoff_uses_published_timing_without_weakening_execution_checks(self) -> None:
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        case = report._case_by_id(manifest, "long_validation_handoff")
        self.assertNotIn("sync_wait_secs", case["prompt"])
        self.assertNotIn("sync_wait_secs", case["validation"])
        self.assertIn("published validation options", case["prompt"])
        self.assertIn("let the Server control synchronous handoff timing", case["prompt"])
        self.assertTrue(case["correctness"]["requires_real_handoff_for_valid_case"])
        self.assertTrue(case["correctness"]["same_execution_must_be_preserved"])
        self.assertTrue(case["correctness"]["redispatch_forbidden"])
        self.assertTrue(case["validation"]["required"])
        self.assertEqual(case["validation"]["command"], "cargo test --lib")
        self.assertEqual(case["validation"]["expected_terminal_result"], "success")

    def _install_manifest(self, repo: Path) -> None:
        target = repo / "scripts/agent_loop_cases.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(
            report.DEFAULT_CASE_MANIFEST.read_text(encoding="utf-8"),
            encoding="utf-8",
        )

    def test_default_selection_contracts_and_source_oracles(self) -> None:
        from scripts import agent_efficiency_benchmark as efficiency

        self.assertEqual(benchmark.DEFAULT_SELECTION_CASES, (
            "selection_single_read", "selection_batch_read",
            "selection_cross_tool_review", "selection_compound_source_context",
        ))
        self.assertEqual(efficiency.CASES, (
            "readonly_review", "guarded_multi_file_edit", "long_validation_handoff",
        ))
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        repo = report.DEFAULT_CASE_MANIFEST.parent.parent
        for case_id in benchmark.DEFAULT_SELECTION_CASES:
            with self.subTest(case_id=case_id):
                case = report._case_by_id(manifest, case_id)
                self.assertEqual(case["experiment"], "tool_selection")
                self.assertEqual(case["target"], "webcodex_repository")
                self.assertTrue(case["correctness"]["workspace_must_remain_clean"])
                self.assertFalse(case["validation"]["required"])
                oracle = benchmark._fixture_oracle(case, repo)
                if case_id == "selection_cross_tool_review":
                    self.assertFalse(oracle["available"])
                else:
                    self.assertTrue(oracle["passed"], oracle)
                annotation = benchmark._annotation(
                    case, variant=report.TOOL_SELECTION_VARIANT,
                    surface=report.TOOL_SELECTION_VARIANT,
                    base_revision="a" * 40, receipt={"status": "pass"}, oracle=oracle,
                )
                # Fixture eligibility and driver status never supply an answer verdict.
                self.assertNotEqual(annotation.get("correctness", {}).get("task_verdict"), "pass")

    def test_compound_selection_oracle_rejects_missing_or_wrong_source(self) -> None:
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        case = report._case_by_id(manifest, "selection_compound_source_context")
        repo = report.DEFAULT_CASE_MANIFEST.parent.parent
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for rel in case["correctness"]["fixture_oracle"]["files"]:
                target = root / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes((repo / rel).read_bytes())
            self.assertTrue(benchmark._fixture_oracle(case, root)["passed"])
            source = root / "src/tool_runtime/search_and_read.rs"
            source.write_text("unrelated implementation\n", encoding="utf-8")
            self.assertFalse(benchmark._fixture_oracle(case, root)["passed"])
            source.unlink()
            self.assertFalse(benchmark._fixture_oracle(case, root)["passed"])

    def test_pair_order_alternates(self) -> None:
        self.assertEqual(benchmark._pair_order(0), ("direct", "code_mode"))
        self.assertEqual(benchmark._pair_order(1), ("code_mode", "direct"))
        self.assertEqual(benchmark._pair_order(2), ("direct", "code_mode"))

    @unittest.skipIf(os.name == "nt", "paired Host driver is explicitly POSIX-only")
    def test_driver_stdin_is_closed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "stdin.txt"
            benchmark._run_driver(
                [
                    sys.executable,
                    "-c",
                    "from pathlib import Path; import sys; Path(sys.argv[1]).write_text(sys.stdin.read())",
                    str(output),
                ],
                cwd=Path(tmp),
                env={},
            )
            self.assertEqual(output.read_text(encoding="utf-8"), "")

    @unittest.skipIf(os.name == "nt", "paired Host driver is explicitly POSIX-only")
    def test_driver_interruption_terminates_owned_process_tree(self) -> None:
        process = mock.Mock()
        process.pid = 12345
        process.wait.side_effect = KeyboardInterrupt
        with (
            mock.patch.object(benchmark.subprocess, "Popen", return_value=process),
            mock.patch.object(benchmark, "_terminate_driver_tree") as terminate,
        ):
            with self.assertRaises(KeyboardInterrupt):
                benchmark._run_driver(
                    [sys.executable, "-c", "pass"],
                    cwd=Path("."),
                    env={},
                )
        terminate.assert_called_once_with(process)

    def test_windows_host_driver_fails_closed_as_unsupported(self) -> None:
        with mock.patch.object(benchmark.os, "name", "nt"):
            with self.assertRaisesRegex(
                benchmark.DriverPlatformUnsupported,
                "POSIX process-group ownership",
            ):
                benchmark._run_driver(
                    [sys.executable, "-c", "pass"],
                    cwd=Path("."),
                    env={},
                )

    def test_runtime_evidence_gate_fails_closed(self) -> None:
        direct = {
            "outer_calls": {"total": 1},
            "canonical_calls": {"total": 1},
            "availability": {"code_mode_composition": {"available": True}},
            "tools": {"outer_by_name": {"read_files": 1}},
        }
        self.assertTrue(benchmark._runtime_evidence_proven(direct, "direct", "direct"))
        direct["availability"]["code_mode_composition"]["available"] = False
        self.assertFalse(benchmark._runtime_evidence_proven(direct, "direct", "direct"))

        code_mode = {
            "outer_calls": {"total": 1},
            "availability": {"code_mode_composition": {"available": True}},
            "composition": {"outer_code_mode_calls": 1},
            "tools": {"outer_by_name": {"execute_mutating_code_mode": 1}},
        }
        self.assertTrue(
            benchmark._runtime_evidence_proven(code_mode, "code_mode", "guarded_edit")
        )
        self.assertFalse(
            benchmark._runtime_evidence_proven(code_mode, "code_mode", "read_only")
        )
        read_only = {
            "outer_calls": {"total": 2},
            "availability": {"code_mode_composition": {"available": True}},
            "composition": {"outer_code_mode_calls": 1},
            "tools": {
                "outer_by_name": {
                    "work_on_project": 1,
                    "execute_code_mode": 1,
                }
            },
        }
        self.assertTrue(
            benchmark._runtime_evidence_proven(read_only, "code_mode", "read_only")
        )
        read_only["outer_calls"]["total"] = 3
        read_only["tools"]["outer_by_name"]["read_tool_manifest"] = 1
        self.assertTrue(
            benchmark._runtime_evidence_proven(read_only, "code_mode", "read_only")
        )
        read_only["tools"]["outer_by_name"]["read_tool_manifest"] = 2
        read_only["outer_calls"]["total"] = 4
        self.assertFalse(
            benchmark._runtime_evidence_proven(read_only, "code_mode", "read_only")
        )
        read_only["tools"]["outer_by_name"]["read_tool_manifest"] = 1
        read_only["outer_calls"]["total"] = 4
        read_only["tools"]["outer_by_name"]["read_files"] = 1
        self.assertFalse(
            benchmark._runtime_evidence_proven(read_only, "code_mode", "read_only")
        )
        del read_only["tools"]["outer_by_name"]["read_files"]
        del read_only["tools"]["outer_by_name"]["read_tool_manifest"]
        read_only["outer_calls"]["total"] = 1
        del read_only["tools"]["outer_by_name"]["work_on_project"]
        self.assertFalse(
            benchmark._runtime_evidence_proven(read_only, "code_mode", "read_only")
        )
        code_mode["tools"]["outer_by_name"]["execute_code_mode"] = 1
        code_mode["composition"]["outer_code_mode_calls"] = 2
        self.assertFalse(
            benchmark._runtime_evidence_proven(code_mode, "code_mode", "guarded_edit")
        )

    def test_long_handoff_requires_one_launch_and_exact_terminal_relation(self) -> None:
        direct = {
            "canonical_calls": {"by_name": {"cargo_test": 1}},
            "job_convergence": {
                "pending_handoff_count": 1,
                "pending_followup_known_count": 1,
                "pending_to_terminal_ms": {"samples": 1},
                "pending_handoff_by_origin_tool": {"cargo_test": 1},
                "pending_followup_known_by_origin_tool": {"cargo_test": 1},
                "pending_to_terminal_ms_by_origin_tool": {
                    "cargo_test": {"samples": 1}
                },
                "selected_terminal_by_origin_tool": {"cargo_test": 1},
            },
        }
        self.assertTrue(benchmark._long_handoff_terminal_proven(direct, "direct"))
        direct["job_convergence"]["selected_terminal_by_origin_tool"] = {}
        self.assertFalse(benchmark._long_handoff_terminal_proven(direct, "direct"))
        direct["job_convergence"]["selected_terminal_by_origin_tool"] = {"cargo_test": 1}
        direct["canonical_calls"]["by_name"]["cargo_test"] = 2
        self.assertFalse(benchmark._long_handoff_terminal_proven(direct, "direct"))

        code_mode = {
            "availability": {"code_mode_composition": {"available": True}},
            "composition": {
                "nested_tool_counts": {"cargo_test": 1},
                "job_handoffs": {"total": 1},
                "consequential_calls": {"total": 1},
                "known_results": {"total": 0},
                "outcome_unknown": {"total": 0},
            },
            "job_convergence": {
                "pending_handoff_count": 1,
                "pending_followup_known_count": 1,
                "pending_to_terminal_ms": {"samples": 1},
                "pending_handoff_by_origin_tool": {
                    "execute_effectful_code_mode": 1
                },
                "pending_followup_known_by_origin_tool": {
                    "execute_effectful_code_mode": 1
                },
                "pending_to_terminal_ms_by_origin_tool": {
                    "execute_effectful_code_mode": {"samples": 1}
                },
                "selected_terminal_by_origin_tool": {
                    "execute_effectful_code_mode": 1
                },
            },
        }
        self.assertTrue(benchmark._long_handoff_terminal_proven(code_mode, "code_mode"))
        code_mode["job_convergence"]["selected_terminal_by_origin_tool"] = {}
        self.assertFalse(
            benchmark._long_handoff_terminal_proven(code_mode, "code_mode")
        )
        code_mode["job_convergence"]["selected_terminal_by_origin_tool"] = {
            "execute_effectful_code_mode": 1
        }
        code_mode["job_convergence"]["pending_handoff_by_origin_tool"] = {
            "run_process": 1
        }
        self.assertFalse(
            benchmark._long_handoff_terminal_proven(code_mode, "code_mode")
        )
        code_mode["job_convergence"]["pending_handoff_by_origin_tool"] = {
            "execute_effectful_code_mode": 1
        }
        code_mode["job_convergence"]["pending_to_terminal_ms"]["samples"] = 0
        self.assertFalse(benchmark._long_handoff_terminal_proven(code_mode, "code_mode"))
        code_mode["job_convergence"]["pending_to_terminal_ms"]["samples"] = 1
        code_mode["composition"]["nested_tool_counts"]["cargo_test"] = 2
        self.assertFalse(benchmark._long_handoff_terminal_proven(code_mode, "code_mode"))

    def test_multi_file_contract_requires_one_edit_search_and_post_read_capacity(self) -> None:
        direct = {
            "canonical_calls": {
                "by_name": {
                    "edit_project_files": 1,
                    "search_project_texts": 1,
                    "read_files": 2,
                }
            },
            "outer_calls": {"failed": 0, "timeout_or_unknown": 0},
            "edit_outcomes": {
                "by_tool": {"edit_project_files": {"applied": 1}},
            },
            "availability": {"edit_outcomes": {"available": True}},
        }
        self.assertTrue(benchmark._guarded_multi_file_contract_proven(direct, "direct"))
        direct["outer_calls"]["failed"] = 1
        self.assertFalse(benchmark._guarded_multi_file_contract_proven(direct, "direct"))
        direct["outer_calls"]["failed"] = 0
        direct["edit_outcomes"]["by_tool"]["edit_project_files"] = {
            "dry_run_would_change": 1
        }
        self.assertFalse(benchmark._guarded_multi_file_contract_proven(direct, "direct"))
        direct["edit_outcomes"]["by_tool"]["edit_project_files"] = {"applied": 1}
        direct["canonical_calls"]["by_name"]["edit_project_files"] = 2
        self.assertFalse(benchmark._guarded_multi_file_contract_proven(direct, "direct"))

        code_mode = {
            "availability": {"code_mode_composition": {"available": True}},
            "composition": {
                "nested_tool_counts": {
                    "edit_project_files": 1,
                    "search_project_texts": 1,
                    "read_files": 2,
                },
                "consequential_calls": {"total": 1},
                "known_results": {"total": 1},
                "job_handoffs": {"total": 0},
                "outcome_unknown": {"total": 0},
                "mutation_state_changed": {"total": 1},
                "mutation_no_change": {"total": 0},
            },
            "outer_calls": {"failed": 0, "timeout_or_unknown": 0},
            "child_calls": {"failed": 0},
        }
        self.assertTrue(benchmark._guarded_multi_file_contract_proven(code_mode, "code_mode"))
        code_mode["child_calls"]["failed"] = 1
        self.assertFalse(benchmark._guarded_multi_file_contract_proven(code_mode, "code_mode"))
        code_mode["child_calls"]["failed"] = 0
        code_mode["composition"]["consequential_calls"]["total"] = 2
        self.assertFalse(benchmark._guarded_multi_file_contract_proven(code_mode, "code_mode"))
        code_mode["composition"]["consequential_calls"]["total"] = 1
        code_mode["composition"]["mutation_state_changed"]["total"] = 0
        self.assertFalse(benchmark._guarded_multi_file_contract_proven(code_mode, "code_mode"))
        code_mode["composition"]["mutation_state_changed"]["total"] = 1
        code_mode["composition"]["nested_tool_counts"]["read_files"] = 1
        self.assertFalse(benchmark._guarded_multi_file_contract_proven(code_mode, "code_mode"))

    def test_fixture_oracle_checks_both_files_and_exact_diff(self) -> None:
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        case = report._case_by_id(manifest, "guarded_multi_file_edit")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-b", "main"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=root, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=root, check=True)
            fixture = root / "tests/fixtures/agent-loop-multi-file"
            fixture.mkdir(parents=True)
            (fixture / "alpha.txt").write_text(
                "BENCH_TARGET_ALPHA=before\nALPHA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
                newline="\n",
            )
            (fixture / "beta.txt").write_text(
                "BENCH_TARGET_BETA=before\nBETA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
                newline="\n",
            )
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            subprocess.run(["git", "commit", "-m", "fixture"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            (fixture / "alpha.txt").write_text(
                "BENCH_TARGET_ALPHA=after\nALPHA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
                newline="\n",
            )
            (fixture / "beta.txt").write_text(
                "BENCH_TARGET_BETA=after\nBETA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
                newline="\n",
            )
            result = benchmark._fixture_oracle(case, root)
            self.assertTrue(result["available"])
            self.assertTrue(result["passed"])

            subprocess.run(["git", "add", "."], cwd=root, check=True)
            staged = benchmark._fixture_oracle(case, root)
            self.assertTrue(staged["passed"])

            (fixture / "beta.txt").write_text(
                "BENCH_TARGET_BETA=after\nBETA_UNRELATED_SENTINEL=lost\n",
                encoding="utf-8",
                newline="\n",
            )
            result = benchmark._fixture_oracle(case, root)
            self.assertFalse(result["passed"])

            (fixture / "beta.txt").write_text(
                "BENCH_TARGET_BETA=after\nBETA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
                newline="\n",
            )
            (root / "unexpected.txt").write_text("unexpected\n", encoding="utf-8")
            result = benchmark._fixture_oracle(case, root)
            self.assertFalse(result["passed"])
            changed_check = next(check for check in result["checks"] if check.get("kind") == "changed_files")
            self.assertIn("unexpected.txt", changed_check["actual"])

            (root / "unexpected.txt").unlink()
            (fixture / "beta.txt").write_bytes(b"\xff\xfe")
            result = benchmark._fixture_oracle(case, root)
            self.assertFalse(result["passed"])
            beta_check = next(check for check in result["checks"] if check.get("path", "").endswith("beta.txt"))
            self.assertEqual(beta_check["reason"], "file is not UTF-8")

            (fixture / "beta.txt").write_bytes(b"x" * (benchmark.MAX_FIXTURE_ORACLE_BYTES + 1))
            result = benchmark._fixture_oracle(case, root)
            self.assertFalse(result["passed"])
            beta_check = next(check for check in result["checks"] if check.get("path", "").endswith("beta.txt"))
            self.assertEqual(beta_check["reason"], "file exceeds oracle byte limit")

    def test_fixture_oracle_reports_git_path_query_failure(self) -> None:
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        case = report._case_by_id(manifest, "guarded_multi_file_edit")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fixture = root / "tests/fixtures/agent-loop-multi-file"
            fixture.mkdir(parents=True)
            (fixture / "alpha.txt").write_text(
                "BENCH_TARGET_ALPHA=after\nALPHA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            (fixture / "beta.txt").write_text(
                "BENCH_TARGET_BETA=after\nBETA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            with mock.patch.object(
                benchmark,
                "_bounded_git_paths",
                side_effect=[(False, False, []), (True, False, [])],
            ):
                result = benchmark._fixture_oracle(case, root)
            changed = next(
                check for check in result["checks"] if check.get("kind") == "changed_files"
            )
            self.assertFalse(changed["passed"])
            self.assertEqual(changed["reason"], "changed path query failed")

    @unittest.skipIf(os.name == "nt", "FIFO is POSIX-only")
    def test_fixture_oracle_rejects_fifo_without_blocking(self) -> None:
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        case = report._case_by_id(manifest, "guarded_multi_file_edit")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-b", "main"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=root, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=root, check=True)
            fixture = root / "tests/fixtures/agent-loop-multi-file"
            fixture.mkdir(parents=True)
            (fixture / "alpha.txt").write_text(
                "BENCH_TARGET_ALPHA=after\nALPHA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            beta = fixture / "beta.txt"
            beta.write_text(
                "BENCH_TARGET_BETA=before\nBETA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            subprocess.run(["git", "commit", "-m", "fixture"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            beta.unlink()
            os.mkfifo(beta)
            started = time.monotonic()
            result = benchmark._fixture_oracle(case, root)
            self.assertLess(time.monotonic() - started, 0.5)
            self.assertFalse(result["passed"])
            beta_check = next(check for check in result["checks"] if check.get("path", "").endswith("beta.txt"))
            self.assertEqual(beta_check["reason"], "file is not regular")

    def test_multi_file_fixture_is_pinned_to_lf_checkout(self) -> None:
        repo = Path(__file__).resolve().parents[2]
        completed = subprocess.run(
            [
                "git",
                "check-attr",
                "eol",
                "--",
                "tests/fixtures/agent-loop-multi-file/alpha.txt",
                "tests/fixtures/agent-loop-multi-file/beta.txt",
            ],
            cwd=repo,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=True,
        )
        lines = [line for line in completed.stdout.splitlines() if line]
        self.assertEqual(len(lines), 2)
        self.assertTrue(all(line.endswith(": lf") for line in lines))

    def test_malformed_receipt_status_is_a_contract_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            receipt = Path(tmp) / "receipt.json"
            receipt.write_text('{"status": []}', encoding="utf-8")
            with self.assertRaisesRegex(benchmark.BenchmarkError, "status must be"):
                benchmark._load_driver_receipt(receipt)

    def test_driver_receipt_depth_has_an_explicit_inclusive_limit(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            receipt = Path(tmp) / "receipt.json"
            for kind in ("array", "object"):
                for depth in (128, 129):
                    with self.subTest(kind=kind, depth=depth):
                        value: object = "private-marker"
                        for _ in range(depth - 1):
                            value = [value] if kind == "array" else {"value": value}
                        document = {"status": "unsupported", "extra": value}
                        receipt.write_text(json.dumps(document), encoding="utf-8")
                        self.assertLess(receipt.stat().st_size, benchmark.MAX_DRIVER_RECEIPT_BYTES)
                        if depth == 128:
                            self.assertEqual(benchmark._load_driver_receipt(receipt), document)
                        else:
                            with self.assertRaisesRegex(
                                benchmark.BenchmarkError,
                                "^driver result is not valid bounded JSON$",
                            ):
                                benchmark._load_driver_receipt(receipt)

    def test_driver_receipt_pathological_json_is_a_contract_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name, raw in (
                ("deep.json", '{"status":"unsupported","extra":' + "[" * 10000 + "0" + "]" * 10000 + "}"),
                ("huge-int.json", '{"status":"unsupported","extra":' + "1" * 10000 + "}"),
            ):
                receipt = root / name
                receipt.write_text(raw, encoding="utf-8")
                # Parser exceptions differ for excessive integers across Python versions.
                pattern = "bounded JSON" if name == "deep.json" else r"not valid (?:bounded )?JSON"
                with self.assertRaisesRegex(benchmark.BenchmarkError, pattern):
                    benchmark._load_driver_receipt(receipt)

    @unittest.skipIf(os.name == "nt", "FIFO is POSIX-only")
    def test_driver_receipt_rejects_fifo_without_blocking(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            receipt = Path(tmp) / "receipt.json"
            os.mkfifo(receipt)
            started = time.monotonic()
            with self.assertRaisesRegex(benchmark.BenchmarkError, "regular file"):
                benchmark._load_driver_receipt(receipt)
            self.assertLess(time.monotonic() - started, 0.5)

    def test_driver_receipt_rejects_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / "target.json"
            target.write_text('{"status":"pass"}', encoding="utf-8")
            receipt = root / "receipt.json"
            try:
                receipt.symlink_to(target)
            except OSError:
                self.skipTest("symlinks unavailable")
            with self.assertRaisesRegex(benchmark.BenchmarkError, "regular file"):
                benchmark._load_driver_receipt(receipt)

    def test_comparison_requires_two_effective_pass_samples(self) -> None:
        summary = {"benchmark": {}}
        self.assertIsNone(
            benchmark._comparison(
                {"status": "partial", "summary": summary},
                {"status": "pass", "summary": summary},
            )
        )
        self.assertIsNone(
            benchmark._comparison(
                {"status": "pass", "summary": None},
                {"status": "pass", "summary": summary},
            )
        )

    def test_paired_delta_summary_uses_only_correctness_qualified_pairs(self) -> None:
        def paired(wall: int, calls: int, result_bytes: int) -> dict:
            return {
                "comparison": {
                    "throughput_compatibility": {"comparable": True},
                    "metrics": [
                        {"metric": "task_wall_time_ms", "baseline": 1000, "candidate": 1000 + wall, "delta": wall, "comparable": True},
                        {"metric": "outer_calls.total", "baseline": 5, "candidate": 5 + calls, "delta": calls, "comparable": True},
                        {"metric": "results.serialized_tool_result_bytes.total", "baseline": 1000, "candidate": 1000 + result_bytes, "delta": result_bytes, "comparable": True},
                    ],
                }
            }

        result = benchmark._paired_delta_summary([
            paired(-300, -2, -400),
            {"comparison": None},
            paired(-100, -1, -200),
            {"comparison": {"throughput_compatibility": {"comparable": False}, "metrics": [
                {"metric": "task_wall_time_ms", "baseline": 1000, "candidate": 1, "delta": -999, "comparable": True},
            ]}},
        ])
        self.assertEqual(result["qualified_pairs"], 2)
        self.assertEqual(result["excluded_pairs"], 2)
        wall = result["candidate_minus_direct"]["task_wall_time_ms"]
        self.assertEqual(
            wall,
            {"total": -400, "observed_total": -400, "p50": -300, "p95": -100, "samples": 2, "missing": 0, "complete": True},
        )
        self.assertEqual(result["candidate_minus_direct"]["outer_calls.total"]["p50"], -2)
        self.assertEqual(result["candidate_minus_direct"]["results.serialized_tool_result_bytes.total"]["p95"], -200)
        repair = result["candidate_minus_direct"]["repair_turns.total"]
        self.assertEqual((repair["samples"], repair["missing"], repair["total"]), (0, 2, None))
        self.assertFalse(repair["complete"])

    def test_paired_delta_summary_missing_and_malformed_metrics_fail_closed(self) -> None:
        valid = {"metric": "task_wall_time_ms", "baseline": 100, "candidate": 90, "delta": -10, "comparable": True}
        comparison = lambda metrics: {
            "comparison": {"throughput_compatibility": {"comparable": True}, "metrics": metrics}
        }
        result = benchmark._paired_delta_summary([
            comparison([valid]),
            comparison([
                {"metric": "task_wall_time_ms", "baseline": True, "candidate": 90, "delta": -10, "comparable": True},
            ]),
            comparison([valid, valid]),
            comparison([
                {"metric": "task_wall_time_ms", "baseline": 100, "candidate": 90, "delta": -5, "comparable": True},
            ]),
        ])
        metric = result["candidate_minus_direct"]["task_wall_time_ms"]
        self.assertEqual((metric["samples"], metric["missing"]), (1, 3))
        self.assertEqual(metric["observed_total"], -10)
        self.assertIsNone(metric["total"])
        self.assertFalse(metric["complete"])
        self.assertEqual(metric["p50"], -10)
        empty = benchmark._paired_delta_summary([{"comparison": None}])
        self.assertEqual(empty["qualified_pairs"], 0)
        self.assertIsNone(empty["candidate_minus_direct"]["task_wall_time_ms"]["total"])
        self.assertIsNone(empty["candidate_minus_direct"]["task_wall_time_ms"]["p50"])
        self.assertFalse(empty["candidate_minus_direct"]["task_wall_time_ms"]["complete"])

    def test_annotation_normalizes_omitted_correctness_verdicts(self) -> None:
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        readonly = report._case_by_id(manifest, "readonly_review")
        annotation = benchmark._annotation(
            readonly,
            variant="direct",
            surface="direct",
            base_revision="a" * 40,
            receipt={"correctness": {"task_verdict": "pass"}},
            oracle={"available": False, "passed": None, "checks": [], "reason": None},
        )
        self.assertEqual(
            annotation["correctness"],
            {"task_verdict": "pass", "validation_verdict": "not_required"},
        )

        validation = report._case_by_id(manifest, "long_validation_handoff")
        annotation = benchmark._annotation(
            validation,
            variant="direct",
            surface="direct",
            base_revision="a" * 40,
            receipt={"correctness": {"task_verdict": "pass"}},
            oracle={"available": False, "passed": None, "checks": [], "reason": None},
        )
        self.assertEqual(
            annotation["correctness"],
            {"task_verdict": "pass", "validation_verdict": "unavailable"},
        )

    def test_relative_receipt_artifacts_are_anchored_to_workspace(self) -> None:
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        case = report._case_by_id(manifest, "readonly_review")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            annotation = root / "annotation.json"
            case_manifest = root / "manifest.json"
            case_manifest.write_text(
                report.DEFAULT_CASE_MANIFEST.read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            receipt = {
                "status": "pass",
                "audit_db": ".webcodex/action_audit.db",
                "trace_root": ".webcodex/traces",
                "workflow_session_id": "wc_sess_test",
            }
            with mock.patch.object(report, "summarize", return_value={"ok": True}) as summarize:
                value = benchmark._summary_for_receipt(
                    case,
                    variant="direct",
                    surface="direct",
                    base_revision="0" * 40,
                    receipt=receipt,
                    annotation_path=annotation,
                    case_manifest=case_manifest,
                    workspace=root,
                )
            self.assertEqual(value, {"ok": True})
            kwargs = summarize.call_args.kwargs
            self.assertEqual(kwargs["audit_db"], root / ".webcodex/action_audit.db")
            self.assertEqual(kwargs["trace_root"], root / ".webcodex/traces")

    def test_fixture_oracle_bounds_changed_path_listing(self) -> None:
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        case = report._case_by_id(manifest, "guarded_multi_file_edit")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-b", "main"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=root, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=root, check=True)
            fixture = root / "tests/fixtures/agent-loop-multi-file"
            fixture.mkdir(parents=True)
            (fixture / "alpha.txt").write_text(
                "BENCH_TARGET_ALPHA=before\nALPHA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            (fixture / "beta.txt").write_text(
                "BENCH_TARGET_BETA=before\nBETA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            subprocess.run(["git", "commit", "-m", "fixture"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            (fixture / "alpha.txt").write_text(
                "BENCH_TARGET_ALPHA=after\nALPHA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            (fixture / "beta.txt").write_text(
                "BENCH_TARGET_BETA=after\nBETA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            overflow = root / "overflow"
            overflow.mkdir()
            for index in range(benchmark.MAX_GIT_PATHS + 1):
                (overflow / f"path-{index:04d}.txt").write_text("x", encoding="utf-8")

            result = benchmark._fixture_oracle(case, root)
            changed = next(
                check for check in result["checks"] if check.get("kind") == "changed_files"
            )
            self.assertFalse(result["passed"])
            self.assertFalse(changed["passed"])
            self.assertEqual(changed["reason"], "changed path listing exceeds oracle limit")
            self.assertIsNone(changed["actual"])

    @unittest.skipIf(os.name == "nt", "paired Host driver requires POSIX process-group ownership")
    def test_missing_receipts_are_retained_as_failed_samples(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
            result = benchmark.run_benchmark(
                repo=repo,
                base_revision=base,
                driver_argv=[sys.executable, "-c", "pass"],
                case_ids=["readonly_review"],
                pairs=1,
            )
            self.assertEqual(result["status_counts"]["fail"], 2)
            for sample in result["cases"][0]["pairs"][0]["samples"]:
                self.assertEqual(sample["reason_code"], "driver_receipt_missing")
                self.assertIsNone(sample["summary"])
            self.assertIsNone(result["cases"][0]["pairs"][0]["comparison"])

    def test_unsupported_driver_samples_are_retained_and_worktrees_cleaned(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()

            driver = root / "driver.py"
            driver.write_text(
                "import json, os\n"
                "from pathlib import Path\n"
                "Path(os.environ['WEBCODEX_BENCH_DRIVER_RESULT']).write_text("
                "json.dumps({'status':'unsupported','message':'fixture driver'}), encoding='utf-8')\n",
                encoding="utf-8",
            )
            result = benchmark.run_benchmark(
                repo=repo,
                base_revision=base,
                driver_argv=[sys.executable, str(driver)],
                case_ids=["readonly_review"],
                pairs=2,
            )
            self.assertEqual(result["sample_count"], 4)
            self.assertEqual(result["status_counts"]["unsupported"], 4)
            self.assertEqual(result["cases"][0]["pairs"][0]["order"], ["direct", "code_mode"])
            self.assertEqual(result["cases"][0]["pairs"][1]["order"], ["code_mode", "direct"])
            self.assertIsNone(result["cases"][0]["pairs"][0]["comparison"])

            worktrees = subprocess.check_output(["git", "worktree", "list", "--porcelain"], cwd=repo, text=True)
            self.assertEqual(worktrees.count("worktree "), 1)

    @unittest.skipIf(os.name == "nt", "post-checkout hook regression is POSIX-only")
    def test_failed_worktree_checkout_rolls_back_owned_registration(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()

            unrelated = root / "unrelated"
            subprocess.run(
                ["git", "worktree", "add", "--detach", str(unrelated), base],
                cwd=repo,
                check=True,
                stdout=subprocess.DEVNULL,
            )
            hook = repo / ".git/hooks/post-checkout"
            hook.write_text("#!/bin/sh\nexit 1\n", encoding="utf-8")
            hook.chmod(0o755)

            result = benchmark.run_benchmark(
                repo=repo,
                base_revision=base,
                driver_argv=[sys.executable, "-c", "pass"],
                case_ids=["readonly_review"],
                pairs=1,
            )
            self.assertEqual(result["status_counts"]["fail"], 2)
            worktrees = subprocess.check_output(
                ["git", "worktree", "list", "--porcelain"],
                cwd=repo,
                text=True,
            )
            self.assertIn(str(unrelated), worktrees)
            self.assertNotIn("webcodex-agent-loop-bench-", worktrees)
            subprocess.run(
                ["git", "worktree", "remove", "--force", str(unrelated)],
                cwd=repo,
                check=True,
            )

    def test_cleanup_does_not_prune_unrelated_worktree(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()

            unrelated = root / "unrelated"
            subprocess.run(["git", "worktree", "add", "--detach", str(unrelated), base], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            driver = root / "driver.py"
            driver.write_text(
                "import json, os\n"
                "from pathlib import Path\n"
                "Path(os.environ['WEBCODEX_BENCH_DRIVER_RESULT']).write_text("
                "json.dumps({'status':'unsupported'}), encoding='utf-8')\n",
                encoding="utf-8",
            )
            benchmark.run_benchmark(
                repo=repo,
                base_revision=base,
                driver_argv=[sys.executable, str(driver)],
                case_ids=["readonly_review"],
                pairs=1,
            )
            status = subprocess.run(
                ["git", "status", "--porcelain"],
                cwd=unrelated,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            self.assertEqual(status.returncode, 0, status.stderr)
            subprocess.run(["git", "worktree", "remove", "--force", str(unrelated)], cwd=repo, check=True)

    def test_manifest_blob_is_bounded_and_utf8(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-b", "main"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=root, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=root, check=True)
            manifest = root / "scripts/agent_loop_cases.json"
            manifest.parent.mkdir(parents=True)
            manifest.write_bytes(b"\xff\xfe")
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            subprocess.run(["git", "commit", "-m", "bad utf8"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
            with self.assertRaisesRegex(benchmark.BenchmarkError, "UTF-8"):
                benchmark._read_manifest_at_revision(root, base)

            manifest.write_bytes(b"x" * (benchmark.MAX_CASE_MANIFEST_BYTES + 1))
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            subprocess.run(["git", "commit", "-m", "too large"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
            with self.assertRaisesRegex(benchmark.BenchmarkError, "1 MiB"):
                benchmark._read_manifest_at_revision(root, base)

    @unittest.skipIf(os.name == "nt", "paired Host driver requires POSIX process-group ownership")
    def test_case_manifest_is_pinned_to_base_revision(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()

            manifest_path = repo / "scripts/agent_loop_cases.json"
            dirty = json.loads(manifest_path.read_text(encoding="utf-8"))
            dirty["cases"][0]["prompt"] = "dirty prompt must not be observed"
            manifest_path.write_text(json.dumps(dirty), encoding="utf-8")

            observed = root / "observed.txt"
            driver = root / "driver.py"
            driver.write_text(
                "import os, sys\n"
                "from pathlib import Path\n"
                "Path(sys.argv[1]).write_text(os.environ['WEBCODEX_BENCH_PROMPT'], encoding='utf-8')\n"
                "Path(os.environ['WEBCODEX_BENCH_DRIVER_RESULT']).write_text('{\"status\":\"unsupported\"}', encoding='utf-8')\n",
                encoding="utf-8",
            )
            benchmark.run_benchmark(
                repo=repo,
                base_revision=base,
                driver_argv=[sys.executable, str(driver), str(observed)],
                case_ids=["readonly_review"],
                pairs=1,
            )
            pinned = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
            expected = report._case_by_id(pinned, "readonly_review")["prompt"]
            self.assertEqual(observed.read_text(encoding="utf-8"), expected)

    def test_base_revision_must_match_checkout_head(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("one\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "one"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            old = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
            (repo / "README.md").write_text("two\n", encoding="utf-8")
            subprocess.run(["git", "commit", "-am", "two"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            with self.assertRaisesRegex(benchmark.BenchmarkError, "current checkout HEAD"):
                benchmark.run_benchmark(
                    repo=repo,
                    base_revision=old,
                    driver_argv=[sys.executable, "-c", "raise SystemExit(0)"],
                    case_ids=["readonly_review"],
                    pairs=1,
                )

    def test_fixture_oracle_uses_recorded_base_when_head_advances(self) -> None:
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        case = report._case_by_id(manifest, "guarded_multi_file_edit")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-b", "main"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=root, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=root, check=True)
            fixture = root / "tests/fixtures/agent-loop-multi-file"
            fixture.mkdir(parents=True)
            (fixture / "alpha.txt").write_text(
                "BENCH_TARGET_ALPHA=before\nALPHA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            (fixture / "beta.txt").write_text(
                "BENCH_TARGET_BETA=before\nBETA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            subprocess.run(["git", "commit", "-m", "base"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
            (fixture / "alpha.txt").write_text(
                "BENCH_TARGET_ALPHA=after\nALPHA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            (fixture / "beta.txt").write_text(
                "BENCH_TARGET_BETA=after\nBETA_UNRELATED_SENTINEL=keep\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            subprocess.run(["git", "commit", "-m", "driver commit"], cwd=root, check=True, stdout=subprocess.DEVNULL)
            oracle = benchmark._fixture_oracle(case, root, base)
            changed = next(check for check in oracle["checks"] if check.get("kind") == "changed_files")
            self.assertTrue(changed["passed"])
            self.assertEqual(changed["actual"], sorted(case["correctness"]["changed_files"]))
            self.assertNotEqual(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(), base)

    @unittest.skipIf(os.name == "nt", "POSIX process-group regression")
    def test_driver_timeout_terminates_descendant_process_group(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            marker = root / "descendant-wrote"
            driver = root / "driver.py"
            driver.write_text(
                "import subprocess, sys, time\n"
                "subprocess.Popen([sys.executable, '-c', "
                "'import pathlib,time,sys; time.sleep(1); pathlib.Path(sys.argv[1]).write_text(\\\"late\\\")', "
                "sys.argv[1]])\n"
                "time.sleep(10)\n",
                encoding="utf-8",
            )
            old_timeout = benchmark.MAX_DRIVER_SECS
            benchmark.MAX_DRIVER_SECS = 0.1
            try:
                with self.assertRaisesRegex(benchmark.BenchmarkError, "exceeded"):
                    benchmark._run_driver(
                        [sys.executable, str(driver), str(marker)],
                        cwd=root,
                        env=dict(os.environ),
                    )
            finally:
                benchmark.MAX_DRIVER_SECS = old_timeout
            time.sleep(1.2)
            self.assertFalse(marker.exists())

    def test_duplicate_case_ids_are_rejected_before_driver_execution(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()

            marker = root / "driver-ran"
            driver = root / "driver.py"
            driver.write_text(
                "from pathlib import Path; import sys; Path(sys.argv[1]).write_text('ran')\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(benchmark.BenchmarkError, "duplicate case ids"):
                benchmark.run_benchmark(
                    repo=repo,
                    base_revision=base,
                    driver_argv=[sys.executable, str(driver), str(marker)],
                    case_ids=["readonly_review", "readonly_review"],
                    pairs=1,
                )
            self.assertFalse(marker.exists())

    @unittest.skipIf(os.name == "nt", "paired Host driver requires POSIX process-group ownership")
    def test_run_sample_clears_stale_receipt_before_driver(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
            manifest = report.load_case_manifest(repo / "scripts/agent_loop_cases.json")
            case = report._case_by_id(manifest, "readonly_review")

            bench_root = root / "bench"
            for name in ("worktrees", "receipts", "annotations"):
                (bench_root / name).mkdir(parents=True, exist_ok=True)
            stale = bench_root / "receipts" / "readonly_review-p1-1-direct.json"
            stale.write_text('{"status":"unsupported"}', encoding="utf-8")
            sample = benchmark._run_sample(
                repo,
                bench_root,
                [sys.executable, "-c", "pass"],
                case,
                variant="direct",
                pair_index=0,
                ordinal=0,
                base_revision=base,
                case_manifest=repo / "scripts/agent_loop_cases.json",
            )
            self.assertEqual(sample["status"], "fail")
            self.assertEqual(sample["reason_code"], "driver_receipt_missing")

    def test_driver_timeout_has_distinct_reason_code(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
            manifest_path = repo / "scripts/agent_loop_cases.json"
            manifest = report.load_case_manifest(manifest_path)
            case = report._case_by_id(manifest, "readonly_review")
            bench_root = root / "bench"
            for name in ("worktrees", "receipts", "annotations"):
                (bench_root / name).mkdir(parents=True, exist_ok=True)

            with mock.patch.object(
                benchmark,
                "_run_driver",
                side_effect=benchmark.DriverTimeoutError("timeout"),
            ):
                sample = benchmark._run_sample(
                    repo,
                    bench_root,
                    [sys.executable, "-c", "pass"],
                    case,
                    variant="direct",
                    pair_index=0,
                    ordinal=0,
                    base_revision=base,
                    case_manifest=manifest_path,
                )
            self.assertEqual(sample["status"], "fail")
            self.assertEqual(sample["reason_code"], "driver_timeout")
            self.assertIsNone(sample["driver_exit_code"])

    @unittest.skipIf(os.name == "nt", "paired Host driver requires POSIX process-group ownership")
    def test_contract_error_preserves_driver_exit_code(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
            manifest = report.load_case_manifest(repo / "scripts/agent_loop_cases.json")
            case = report._case_by_id(manifest, "readonly_review")
            bench_root = root / "bench"
            for name in ("worktrees", "receipts", "annotations"):
                (bench_root / name).mkdir(parents=True, exist_ok=True)
            driver = root / "driver.py"
            driver.write_text(
                "import os\n"
                "from pathlib import Path\n"
                "Path(os.environ['WEBCODEX_BENCH_DRIVER_RESULT']).write_text('not-json', encoding='utf-8')\n"
                "raise SystemExit(7)\n",
                encoding="utf-8",
            )
            sample = benchmark._run_sample(
                repo,
                bench_root,
                [sys.executable, str(driver)],
                case,
                variant="direct",
                pair_index=0,
                ordinal=0,
                base_revision=base,
                case_manifest=repo / "scripts/agent_loop_cases.json",
            )
            self.assertEqual(sample["status"], "fail")
            self.assertEqual(sample["reason_code"], "sample_contract_error")
            self.assertEqual(sample["driver_exit_code"], 7)

    @unittest.skipIf(os.name == "nt", "paired Host driver requires POSIX process-group ownership")
    def test_fallback_task_timing_starts_after_worktree_setup(self) -> None:
        self._check_task_timing_after_setup(explicit_null=False)

    @unittest.skipIf(os.name == "nt", "paired Host driver requires POSIX process-group ownership")
    def test_explicit_unknown_task_timing_excludes_manual_wait(self) -> None:
        self._check_task_timing_after_setup(explicit_null=True)

    def _check_task_timing_after_setup(self, *, explicit_null: bool) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
            manifest_path = repo / "scripts/agent_loop_cases.json"
            manifest = report.load_case_manifest(manifest_path)
            case = report._case_by_id(manifest, "readonly_review")
            bench_root = root / "bench"
            for name in ("worktrees", "receipts", "annotations"):
                (bench_root / name).mkdir(parents=True, exist_ok=True)
            driver = root / "driver.py"
            driver_receipt = {"status": "unsupported"}
            if explicit_null:
                driver_receipt["task_timing"] = None
            driver.write_text(
                "import os\n"
                "assert 'WEBCODEX_BENCH_HOST_CELL_POLICY' not in os.environ\n"
                "from pathlib import Path\n"
                f"Path(os.environ['WEBCODEX_BENCH_DRIVER_RESULT']).write_text({json.dumps(driver_receipt)!r}, encoding='utf-8')\n",
                encoding="utf-8",
            )

            setup_done_ms: list[int] = []
            original = benchmark._worktree_add

            def recorded_add(repo_arg: Path, path_arg: Path, revision_arg: str) -> None:
                original(repo_arg, path_arg, revision_arg)
                setup_done_ms.append(time.time_ns() // 1_000_000)

            benchmark._worktree_add = recorded_add
            try:
                with mock.patch.dict(os.environ, {"WEBCODEX_BENCH_HOST_CELL_POLICY": "host_single_call"}):
                    sample = benchmark._run_sample(
                        repo,
                        bench_root,
                        [sys.executable, str(driver)],
                        case,
                        variant="direct",
                        pair_index=0,
                        ordinal=0,
                        base_revision=base,
                        case_manifest=manifest_path,
                    )
            finally:
                benchmark._worktree_add = original

            self.assertEqual(sample["driver_exit_code"], 0)
            annotation = json.loads(
                (bench_root / "annotations" / "readonly_review-p1-1-direct.json").read_text(encoding="utf-8")
            )
            if explicit_null:
                self.assertIsNone(annotation.get("task_timing"))
            else:
                self.assertGreaterEqual(annotation["task_timing"]["started_at_ms"], setup_done_ms[0])

    def test_non_repository_case_target_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
            with self.assertRaisesRegex(benchmark.BenchmarkError, "supports only repository/disposable generic targets"):
                benchmark.run_benchmark(
                    repo=repo,
                    base_revision=base,
                    driver_argv=[sys.executable, "-c", "pass"],
                    case_ids=["focused_edit_validation"],
                    pairs=1,
                )

    def test_repo_argument_is_canonicalized_from_subdirectory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
            driver = root / "driver.py"
            driver.write_text(
                "import os\n"
                "from pathlib import Path\n"
                "Path(os.environ['WEBCODEX_BENCH_DRIVER_RESULT']).write_text('{\"status\":\"unsupported\"}', encoding='utf-8')\n",
                encoding="utf-8",
            )
            result = benchmark.run_benchmark(
                repo=repo / "scripts",
                base_revision=base,
                driver_argv=[sys.executable, str(driver)],
                case_ids=["readonly_review"],
                pairs=1,
            )
            self.assertEqual(result["status_counts"]["unsupported"], 2)

    def test_cli_prints_human_summary_and_json_output(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-b", "main"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "config", "user.email", "bench@test.local"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Bench"], cwd=repo, check=True)
            (repo / "README.md").write_text("fixture\n", encoding="utf-8")
            self._install_manifest(repo)
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            base = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
            driver = root / "driver.py"
            driver.write_text(
                "import json, os\n"
                "from pathlib import Path\n"
                "Path(os.environ['WEBCODEX_BENCH_DRIVER_RESULT']).write_text("
                "json.dumps({'status':'unsupported'}), encoding='utf-8')\n",
                encoding="utf-8",
            )
            output = root / "result.json"
            completed = subprocess.run(
                [
                    sys.executable,
                    str(Path(benchmark.__file__)),
                    "--repo",
                    str(repo),
                    "--base-revision",
                    base,
                    "--driver",
                    f"{sys.executable} {driver}",
                    "--case-id",
                    "readonly_review",
                    "--pairs",
                    "1",
                    "--output",
                    str(output),
                ],
                cwd=Path(__file__).resolve().parents[2],
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            strict = subprocess.run([*completed.args, "--strict"], cwd=Path(__file__).resolve().parents[2],
                text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
            self.assertEqual(strict.returncode, 1, strict.stderr)
            self.assertIn("Agent Loop benchmark", completed.stdout)
            value = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(value["status_counts"]["unsupported"], 2)
            paired = value["cases"][0]["paired_delta_summary"]
            self.assertEqual(paired["qualified_pairs"], 0)
            self.assertEqual(paired["excluded_pairs"], 1)
            self.assertIsNone(
                paired["candidate_minus_direct"]["task_wall_time_ms"]["total"]
            )
            self.assertIn("paired_wall_p50_delta_ms=unavailable", completed.stdout)


class GenericBenchmarkTargetTests(unittest.TestCase):
    def test_all_generic_cases_get_fresh_real_directories_and_cleanup(self):
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in ("worktrees", "receipts", "annotations"):
                (root / name).mkdir()
            observed = root / "observed.jsonl"
            driver = root / "driver.py"
            driver.write_text("""import json, os
from pathlib import Path
root = Path.cwd()
case = os.environ['WEBCODEX_BENCH_CASE_ID']
assert not (root / '.git').exists()
assert not (root / 'leak.txt').exists()
if case == 'generic_encoded_csv':
    assert (root / 'a.csv').read_bytes() == b'name,value\\nAlice,2\\nBob,3\\n'
    assert (root / 'b.csv').read_text(encoding='utf-16').startswith('姓名,数量')
if case == 'generic_mixed_files_conflict':
    assert (root / 'input/pixel.png').read_bytes().startswith(b'\\x89PNG')
if case == 'generic_stale_read_revision':
    assert (root / 'stale.txt').read_bytes() == b'task value: before\\n'
(root / 'leak.txt').write_text('owned temporary mutation')
with open(os.environ['OBSERVED'], 'a') as f:
    f.write(json.dumps({'case':case,'path':str(root)}) + '\\n')
Path(os.environ['WEBCODEX_BENCH_DRIVER_RESULT']).write_text('{"status":"unsupported"}')
""", encoding="utf-8")
            with mock.patch.dict(os.environ, {"OBSERVED": str(observed)}):
                for case_id in sorted(benchmark.GENERIC_CASES):
                    case = report._case_by_id(manifest, case_id)
                    for pair in range(2):
                        sample = benchmark._run_sample(root, root, [sys.executable, str(driver)], case,
                            variant="direct", pair_index=pair, ordinal=0, base_revision="a" * 40,
                            case_manifest=report.DEFAULT_CASE_MANIFEST)
                        self.assertEqual(sample['status'], 'unsupported', sample)
            observations = [json.loads(line) for line in observed.read_text().splitlines()]
            self.assertEqual(len(observations), 12)
            self.assertEqual(len({item['path'] for item in observations}), 12)
            self.assertTrue(all(not Path(item['path']).exists() for item in observations))
            self.assertEqual(list((root / 'worktrees').iterdir()), [])
            self.assertFalse(any(p.is_dir() for p in (root / 'receipts').iterdir()))


    def test_stale_fixture_coordinator_checks_bytes_before_allowing_recovery(self):
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        case = report._case_by_id(manifest, 'generic_stale_read_revision')
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in ('worktrees', 'receipts', 'annotations'):
                (root / name).mkdir()
            driver = root / 'driver.py'
            driver.write_text("""import os, time
from pathlib import Path
coord = Path(os.environ['WEBCODEX_BENCH_FIXTURE_COORDINATION'])
path = Path('stale.txt')
assert path.read_bytes() == b'task value: before\\n'
(coord / 'first-read').touch()
for marker in ('external-written', 'rejection-checked'):
    deadline = time.monotonic() + 3
    while not (coord / marker).exists():
        assert time.monotonic() < deadline
        time.sleep(.01)
    if marker == 'external-written':
        assert path.read_bytes().endswith(b'EXTERNAL_WRITER_SENTINEL_c741\\n')
        (coord / 'stale-rejected').touch()
path.write_bytes(path.read_bytes().replace(b'before', b'after'))
Path(os.environ['WEBCODEX_BENCH_DRIVER_RESULT']).write_text('{"status":"unsupported"}')
""")
            sample = benchmark._run_sample(root, root, [sys.executable, str(driver)], case,
                variant='direct', pair_index=0, ordinal=0, base_revision='a'*40,
                case_manifest=report.DEFAULT_CASE_MANIFEST)
            self.assertEqual(sample['status'], 'unsupported', sample)
            self.assertTrue(sample['fixture_oracle']['passed'])
            self.assertTrue(sample['fixture_oracle']['temporal_evidence_required'])
            self.assertFalse(any(thread.name.startswith('fixture') for thread in benchmark.threading.enumerate()))
            self.assertEqual(list((root/'worktrees').iterdir()), [])

    def test_generic_temporal_requirements_cannot_be_replaced_by_driver_claims(self):
        for case_id in benchmark.GENERIC_CASES:
            expected = False
            self.assertEqual(benchmark._generic_temporal_evidence_proven({'id':case_id}, {'claimed':True}, 'direct'), expected)


    def _strict_generic_matrix(self, case_id, oracle):
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        case = report._case_by_id(manifest, case_id)
        samples = []
        for variant in benchmark.VARIANTS:
            surface = benchmark._surface(case, variant)
            summary = {'outer_calls': {'total': 1}, 'canonical_calls': {'total': 1},
                       'availability': {'code_mode_composition': {'available': True}},
                       'correctness': {'task_verdict': 'pass', 'validation_verdict': 'pass'}}
            if variant == 'code_mode':
                summary['composition'] = {'outer_code_mode_calls': 1}
                summary['tools'] = {'outer_by_name': {benchmark.CODE_MODE_TOOL_BY_SURFACE[surface]: 1}}
            self.assertTrue(benchmark._runtime_evidence_proven(summary, variant, surface))
            sample = {'case_id': case_id, 'variant': variant, 'surface': surface,
                      'status': 'pass', 'summary': summary}
            if oracle is not None:
                sample['fixture_oracle'] = oracle
            samples.append(sample)
        return {'cases': [{'case_id': case_id, 'pairs': [{'pair_index': 0, 'samples': samples}]}]}

    def test_strict_generic_file_and_csv_cases_require_available_passing_oracle(self):
        for case_id in ('generic_mixed_files_conflict', 'generic_encoded_csv'):
            for oracle in (None, {}, {'available': False, 'passed': True},
                           {'available': True, 'passed': False}, {'available': True, 'passed': True}):
                with self.subTest(case=case_id, oracle=oracle):
                    result = self._strict_generic_matrix(case_id, oracle)
                    expected = False  # A final-file oracle alone is never runtime proof.
                    self.assertEqual(benchmark._strict_passed(result, [case_id], 1), expected)

    def test_all_six_strict_proofs_bind_each_sample_own_fixture_without_pair_root_equality(self):
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        for case_id in benchmark.GENERIC_CASES:
            result = self._strict_generic_matrix(case_id, {"available": True, "passed": True})
            for index, sample in enumerate(result["cases"][0]["pairs"][0]["samples"]):
                token = str(index + 1) * 64
                sample["fixture_oracle"] = dict(sample["fixture_oracle"], fixture_identity=token)
                sample["summary"]["benchmark"] = {"base_revision": "a" * 40}
                sample["temporal_evidence"] = {"schema_version": benchmark.temporal.PROOF_SCHEMA_VERSION,
                    "fixture_identity": token, "case_id": case_id, "base_revision": "a" * 40,
                    "case_fingerprint": report._case_fingerprint(report._case_by_id(manifest, case_id)),
                    "available": True, "passed": True,
                    "checks": {name: True for name in benchmark.temporal.CHECK_NAMES[case_id]}}
            self.assertTrue(benchmark._strict_passed(result, [case_id], 1))
            result["cases"][0]["pairs"][0]["samples"][0]["temporal_evidence"]["schema_version"] = 1
            self.assertFalse(benchmark._strict_passed(result, [case_id], 1))

    def test_bootstrap_audit_and_driver_written_correct_files_cannot_pass_no_mock(self):
        from scripts.tests.test_agent_loop_report import AgentLoopReportTests
        audit = AgentLoopReportTests()
        audit.setUp()
        self.addCleanup(audit.tearDown)
        audit.insert_event("bootstrap", tool="work_on_project", trace_id="11111111-1111-4111-8111-111111111111")
        manifest = report.load_case_manifest(report.DEFAULT_CASE_MANIFEST)
        root = audit.root / "benchmark"
        for name in ("worktrees", "receipts", "annotations"):
            (root / name).mkdir(parents=True)
        driver = root / "driver.py"
        driver.write_text("""import json,os,shutil
from pathlib import Path
root=Path.cwd()
if os.environ['WEBCODEX_BENCH_CASE_ID']=='generic_mixed_files_conflict':
    for source in (root/'input').iterdir():
        shutil.copyfile(source, root/'sorted'/('note-2.txt' if source.name=='note.txt' else source.name))
else:
    (root/'merged.csv').write_bytes('name,value\\nAlice,2\\nBob,3\\n陈,5\\n丁,7\\n'.encode())
    (root/'report.json').write_text('{"count":4,"total":17}')
Path(os.environ['WEBCODEX_BENCH_DRIVER_RESULT']).write_text(json.dumps({
    'status':'pass','audit_db':AUDIT,'workflow_session_id':'wc_sess_test',
    'correctness':{'task_verdict':'pass','validation_verdict':'pass'}}))
""".replace("AUDIT", repr(str(audit.audit_db))))
        for case_id in ("generic_mixed_files_conflict", "generic_encoded_csv"):
            case = report._case_by_id(manifest, case_id)
            samples = []
            for ordinal, variant in enumerate(("direct", "host_code_mode")):
                sample = benchmark._run_sample(root, root, [sys.executable, "-B", str(driver)], case,
                    variant=variant, pair_index=0, ordinal=ordinal, base_revision="a" * 40,
                    case_manifest=report.DEFAULT_CASE_MANIFEST)
                self.assertTrue(sample["fixture_oracle"]["passed"], sample)
                self.assertEqual(sample["summary"]["canonical_calls"]["by_name"], {"work_on_project": 1})
                self.assertEqual(sample["status"], "partial", sample)
                samples.append(sample)
            result = {"candidate_variant": "host_code_mode", "cases": [{"case_id": case_id,
                       "pairs": [{"pair_index": 0, "samples": samples}]}]}
            self.assertFalse(benchmark._strict_passed(result, [case_id], 1))

    def test_strict_generic_temporal_gate_rejects_forged_pass_with_passing_oracle(self):
        for case_id in ('generic_failure_repair', 'generic_pending_independent_join',
                        'generic_stale_read_revision', 'generic_missing_output'):
            with self.subTest(case=case_id):
                result = self._strict_generic_matrix(case_id, {'available': True, 'passed': True,
                    'temporal_evidence_required': False, 'temporal_evidence_proven': True})
                self.assertFalse(benchmark._strict_passed(result, [case_id], 1))

    def test_strict_rejects_partial_unsupported_missing_cases_repetitions_and_evidence(self):
        summary = {'outer_calls': {'total': 1}, 'canonical_calls': {'total': 1},
                   'availability': {'code_mode_composition': {'available': True}},
                   'correctness': {'task_verdict': 'pass', 'validation_verdict': 'pass'}}
        sample = {'case_id':'selection_single_read', 'variant':'free_selection', 'surface':'free_selection', 'status':'pass',
                  'summary':summary, 'fixture_oracle':{'available':False}}
        # Use the canonical unrestricted-selection identity from the reporter.
        sample['variant'] = sample['surface'] = report.TOOL_SELECTION_VARIANT
        result = {'experiment':'tool_selection', 'cases':[{'case_id':'selection_single_read', 'samples':[sample]}]}
        self.assertTrue(benchmark._strict_passed(result, ['selection_single_read'], 1))
        self.assertFalse(benchmark._strict_passed(result, ['selection_single_read', 'missing'], 1))
        self.assertFalse(benchmark._strict_passed(result, ['selection_single_read'], 2))
        for status in ('partial', 'unsupported', 'fail'):
            sample['status'] = status
            self.assertFalse(benchmark._strict_passed(result, ['selection_single_read'], 1))
        sample['status'] = 'pass'
        sample['variant'] = 'direct'
        self.assertFalse(benchmark._strict_passed(result, ['selection_single_read'], 1))
        sample['variant'] = report.TOOL_SELECTION_VARIANT
        result['cases'][0]['case_id'] = sample['case_id'] = 'selection_session_recovery'
        summary['correctness']['validation_verdict'] = 'not_required'
        self.assertFalse(benchmark._strict_passed(result, ['selection_session_recovery'], 1))
        result['cases'][0]['case_id'] = sample['case_id'] = 'selection_single_read'
        sample['summary'] = None
        self.assertFalse(benchmark._strict_passed(result, ['selection_single_read'], 1))


if __name__ == "__main__":
    unittest.main()
