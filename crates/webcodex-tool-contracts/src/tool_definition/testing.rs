use super::RunnerCapabilityRequirement::{OwnerOnly, Shell};
use super::ToolVisibility::ModelVisible;
use super::{
    adaptive_runtime_direct, captures_validation_output, def, model_spec, ToolDefinition,
    TOOL_CATEGORY_VALIDATION,
};
use crate::metadata::{
    ToolPathHint::None as NoPath, ToolRisk::JobRun, JOB_RUN, TOOL_PROVIDER_RUNNER,
};

pub(super) const DEFINITIONS: &[ToolDefinition] = &[
    captures_validation_output(model_spec(
        def(
            "cargo_fmt",
            super::ToolAuditPolicy::TYPED_CANONICAL
                .execution(super::ToolAuditExecutionPolicy::TEXT),
            ModelVisible,
            TOOL_CATEGORY_VALIDATION,
            Some(Shell),
            TOOL_PROVIDER_RUNNER,
            super::ToolSemanticContract {
                effect: super::ToolEffect::Execute,
                risk: JobRun,
                approval: super::ToolApprovalPolicy::Standard,
                idempotency: super::ToolIdempotency::NonIdempotent,
            },
            Some(JOB_RUN),
            true,
            NoPath,
            true,
            false,
            super::ToolSessionEvidencePolicy::NONE.validation_identity(super::ToolValidationIdentityKind::CargoFmt),
        ),
        "Use check=false (default) for intentional final formatting after relevant Rust source stabilizes: precheck first, mutate only for a proven rustfmt diff, and use changed/state_changed instead of reproducing rustfmt diffs with edit tools. Do not use cargo_fmt as a per-edit ritual. Use check=true for read-only formatting validation; only check mode may hand off the same execution as a Job. If check mode returns execution_state=pending, keep its continuation as fallback, continue independent work, and let ordinary same-Window/Project/Session results surface sparse terminal attention; do not poll. Observe only for details/recovery. If covered Rust source changes, check=true evidence is stale. For final formatting proof, freeze covered source or rerun after any required source edit. Server timing policy controls same-execution Job handoff grace.",
    )
    .with_execution(super::ToolExecutionContract::new(
        super::ToolExecutionForm::StructuredValidation,
        super::ToolExecutionLifetime::Runner,
        super::ToolExecutionStart::SyncFirst,
        super::ToolExecutionContinuation::ObserveJobs,
    ))),
    captures_validation_output(model_spec(
        def(
            "cargo_check",
                super::ToolAuditPolicy::TYPED_CANONICAL
                    .execution(super::ToolAuditExecutionPolicy::TEXT),
                ModelVisible,
                TOOL_CATEGORY_VALIDATION,
                Some(Shell),
                TOOL_PROVIDER_RUNNER,
                super::ToolSemanticContract {
                    effect: super::ToolEffect::Execute,
                    risk: JobRun,
                    approval: super::ToolApprovalPolicy::Standard,
                    idempotency: super::ToolIdempotency::NonIdempotent,
                },
                Some(JOB_RUN),
                true,
                NoPath,
                false,
                false,
                super::ToolSessionEvidencePolicy::NONE.validation_identity(super::ToolValidationIdentityKind::CargoCheck),
            )
            .with_composition_policy(super::ToolCompositionPolicy::Sequential)
            .with_host_orchestration_hint(
                super::ToolHostOrchestrationHint::sequential()
                    .with_native_batch_field("packages"),
            ),
            "Structured cargo check (default --all-targets) for common supported validation with parsed diagnostics, validation identity, and same execution Job handoff. Use package for one workspace package or packages for a known set; packages runs one Cargo invocation with repeated -p after deterministic sort/dedup. If pending, keep its continuation and continue independent work; same-scope results may surface terminal attention. Observe only for logs/details/recovery; do not poll. Development validation can overlap independent work, but edits to covered source make it stale. For final evidence freeze covered source; if it changes, rerun the appropriate check. Server timing policy controls same-execution Job handoff grace and never changes timeout or retry semantics. Complete synchronous success omits zero counts and empty parser bookkeeping; positive warnings and diagnostics remain. source_state never certifies current workspace source.",
        ).with_discovery_keywords(&["rust", "compile", "typecheck", "compilation", "check", "编译", "检查", "类型"])
        .with_execution(super::ToolExecutionContract::new(
            super::ToolExecutionForm::StructuredValidation,
            super::ToolExecutionLifetime::Runner,
            super::ToolExecutionStart::SyncFirst,
            super::ToolExecutionContinuation::ObserveJobs,
        ))),
    captures_validation_output(model_spec(
        def(
            "cargo_test",
                super::ToolAuditPolicy::TYPED_CANONICAL
                    .execution(super::ToolAuditExecutionPolicy::TEST_ASSERTIONS),
                ModelVisible,
                TOOL_CATEGORY_VALIDATION,
                Some(Shell),
                TOOL_PROVIDER_RUNNER,
                super::ToolSemanticContract {
                    effect: super::ToolEffect::Execute,
                    risk: JobRun,
                    approval: super::ToolApprovalPolicy::Standard,
                    idempotency: super::ToolIdempotency::NonIdempotent,
                },
                Some(JOB_RUN),
                true,
                NoPath,
                false,
                false,
                super::ToolSessionEvidencePolicy::NONE.validation_identity(super::ToolValidationIdentityKind::CargoTest),
            )
            .with_composition_policy(super::ToolCompositionPolicy::Sequential)
            .with_host_orchestration_hint(super::ToolHostOrchestrationHint::sequential()),
            "Structured cargo test for common supported validation with bounded output, executed-test evidence, min_tests/require_tests, validation identity, and same execution Job handoff. lib=true selects Cargo --lib; filter is one Rust substring, not flags such as --exact or --nocapture. Exit 0 alone is not test proof: default requires positive counts; require_tests=false accepts zero only without min_tests; require_tests=true/min_tests enforce a proven minimum. no_run=true is compile-only. Sparse synchronous proof retains tests_run_count and explicit minimum_tests, require_tests=false for accepted zero, or no_run=true for compile-only. Rich success may lack test proof. source_state never certifies current workspace source. If pending, keep its continuation and continue independent work; same-scope results may surface terminal attention. Observe only for logs/details/recovery; do not poll. Edits to covered source stale evidence. For final evidence freeze covered source; if it changes, rerun the appropriate test.",
        ).with_discovery_keywords(&["rust", "unit", "integration", "tests", "test", "suite", "测试", "单元", "集成", "用例"])
        .with_execution(super::ToolExecutionContract::new(
            super::ToolExecutionForm::StructuredValidation,
            super::ToolExecutionLifetime::Runner,
            super::ToolExecutionStart::SyncFirst,
            super::ToolExecutionContinuation::ObserveJobs,
        ))),
    adaptive_runtime_direct(
        captures_validation_output(model_spec(
            def(
                "project_validate",
                super::ToolAuditPolicy::TYPED_CANONICAL
                    .execution(super::ToolAuditExecutionPolicy::TEST_ASSERTIONS),
                ModelVisible,
                TOOL_CATEGORY_VALIDATION,
                Some(OwnerOnly),
                TOOL_PROVIDER_RUNNER,
                super::ToolSemanticContract {
                    effect: super::ToolEffect::Execute,
                    risk: JobRun,
                    approval: super::ToolApprovalPolicy::Standard,
                    idempotency: super::ToolIdempotency::NonIdempotent,
                },
                Some(JOB_RUN),
                true,
                NoPath,
                false,
                false,
                super::ToolSessionEvidencePolicy::NONE.validation_identity(super::ToolValidationIdentityKind::Project),
            )
            .with_composition_policy(super::ToolCompositionPolicy::Sequential)
            .with_host_orchestration_hint(super::ToolHostOrchestrationHint::sequential()),
            "Runner-owned project checks and tests. Rust/Go: format_check/check/test; Python: pytest test, Ruff check/format_check. Node >=22.3: check via node --run (check > typecheck > lint; no pre/post), native TAP test when scripts.test equals node --test or node --test --test-reporter=tap (fixed Runner argv). Direct vitest/vitest run/jest: fixed installed CLI, bounded official per-Job JSON reports (requires project_validation_node_json_v1); logs never prove counts. Other scripts: structured_test_report_unavailable; use ordinary recipes. Project scripts may modify files or use network; success proves script outcome only. scope: packages (1..8) or all_packages=true for Rust/Go check/test; scoped formatting rejected. Rust all-packages needs Runner-proven Project-root Cargo workspace; Go keeps single-module ./.... Python/Node reject scope. test.filter: Rust substring, Go -run, pytest -k; Node TAP rejects filters, as do Vitest/Jest. require_tests/min_tests need proven counts. Python uses existing configured/profile/PATH Python 3 and pytest/Ruff; never installs/falls back. Ruff needs local pyproject.toml [tool.ruff], explicit target-version, no extend; no fixes/cache/bytecode/output files. Missing environment is unavailable. dependency_policy.mode=locked: Cargo --locked/Go -mod=readonly, not offline; Python/Node/formatting reject it. Go forces GO111MODULE=on/GOWORK=off. No arbitrary executable, argv or shell. Long work keeps the same Job; observe pending, never retry. Advanced: cargo_*/go_test.",    ).with_discovery_keywords(&["project", "validation", "build", "tests", "lint", "verify", "项目", "验证", "构建", "测试", "检查"])
        .with_execution(super::ToolExecutionContract::new(
            super::ToolExecutionForm::StructuredValidation,
            super::ToolExecutionLifetime::Runner,
            super::ToolExecutionStart::SyncFirst,
            super::ToolExecutionContinuation::ObserveJobs,
        ))),
        95,
        super::ToolDirectReason::CoreWorkflow,
    ),
    captures_validation_output(model_spec(
            def(
                "go_test",
                super::ToolAuditPolicy::TYPED_CANONICAL
                    .execution(super::ToolAuditExecutionPolicy::TEST_COUNTS),
                ModelVisible,
                TOOL_CATEGORY_VALIDATION,
                Some(OwnerOnly),
                TOOL_PROVIDER_RUNNER,
                super::ToolSemanticContract {
                    effect: super::ToolEffect::Execute,
                    risk: JobRun,
                    approval: super::ToolApprovalPolicy::Standard,
                    idempotency: super::ToolIdempotency::NonIdempotent,
                },
                Some(JOB_RUN),
                true,
                NoPath,
                false,
                false,
                super::ToolSessionEvidencePolicy::NONE.validation_identity(super::ToolValidationIdentityKind::GoTest),
            ),
            "Structured option for common supported go test -json (default ./...) when bounded package scopes, Go JSON test-count evidence, validation identity, or same-execution pending handoff are useful. If execution_state=pending, keep its continuation as fallback and continue independent work; ordinary same-Window/Project/Session results may surface sparse terminal validation truth. Observe only when logs/details/recovery are needed. Validation intent is intrinsic to this validator and Runtime-derived; do not pass generic execution purpose. Requires Runner Go JSON validation support; Server timing policy controls only synchronous grace before the same execution is returned pending and never changes total timeout or retry semantics. Sparse synchronous success retains tests_run_count only when complete evidence proves all counted tests passed; skipped or incomplete evidence stays rich. source_state independently describes source coverage.",
    ).with_discovery_keywords(&["golang", "unit", "integration", "tests", "test", "suite", "测试", "单元", "集成", "用例"])
    .with_execution(super::ToolExecutionContract::new(
        super::ToolExecutionForm::StructuredValidation,
        super::ToolExecutionLifetime::Runner,
        super::ToolExecutionStart::SyncFirst,
        super::ToolExecutionContinuation::ObserveJobs,
    ))),
];
