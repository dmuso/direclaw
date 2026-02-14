# Phase 15 Review: Typed Workflow Inputs and Output Contracts

## Findings

1. **High: Step output contract is still representable as missing fields in config YAML (spec mismatch).**  
   The spec requires that config can no longer represent steps missing contract fields, but the current model still deserializes missing `outputs`/`output_files` into empty collections via `#[serde(default)]`, and tests codify that behavior. This keeps legacy "presence check" semantics instead of making the contract structurally required at the config boundary.  
   - Evidence: `src/config.rs:460`, `src/config.rs:462`, `src/config.rs:1306`  
   - Spec reference: `docs/build/config-typing-and-type-driven-setup-tui-plan.md:58`  
   - Action: Remove default-deserialize behavior for `WorkflowStepConfig.outputs` and `WorkflowStepConfig.output_files` (or gate it behind explicit migration logic), then update tests to assert missing fields fail to load with clear migration guidance.

2. **Medium: Output contract remains stringly typed in the core config model (partial implementation vs planned typed model).**  
   The plan’s target model is `Vec<OutputKey>` and `BTreeMap<OutputKey, PathTemplate>`, but current fields are still `Vec<String>` and `BTreeMap<String, String>`. This means key/path typing and normalization are not enforced by the model itself and must be re-validated in downstream logic.  
   - Evidence: `src/config.rs:461`, `src/config.rs:463`  
   - Spec reference: `docs/build/config-typing-and-type-driven-setup-tui-plan.md:50`, `docs/build/config-typing-and-type-driven-setup-tui-plan.md:51`  
   - Action: Introduce typed output key/path wrappers in `WorkflowStepConfig` and move parse/validation rules into those types so serde/model construction enforces contract invariants directly.

## Notes

- Workflow inputs typing work is directionally aligned: typed wrapper exists, normalization is centralized, and transitional legacy mapping deserialization is covered.
