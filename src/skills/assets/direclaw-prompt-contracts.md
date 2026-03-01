---
name: direclaw-prompt-contracts
description: "Use when editing selector/workflow prompts to keep output contracts deterministic and file-based."
---

# DireClaw Prompt and Output Contracts

## Purpose
Ensure prompt templates produce structured outputs in exactly the files the workflow expects.

## DireClaw Context
- Agent tools (`codex`/`claude`) can emit noisy stdout.
- Structured workflow outputs must be written to explicit file paths.
- Workflow progression depends on downstream steps reading expected output files.
- Prompt wording must remove ambiguity about where structured output is written.

## What To Do
1. Identify the target prompt template and its consuming workflow step/output mapping.
2. Edit instructions so structured output is written to one exact file path per contract.
3. Ensure the prompt explicitly separates:
   - general tool stdout (non-contractual)
   - required structured output file content (contractual)
4. Verify placeholders/variables are valid for the runtime template context.
5. Ensure output schema instructions are stable and deterministic.

## Where To Change
- Prompt/template definitions in orchestrator or workflow config (where applicable):
  - `<current_working_directory>/orchestrator.yaml`
  - workflow template/prompt files referenced by config
- Output contract linkage:
  - workflow `outputs`
  - workflow `output_files`

## Valid Prompt Contract Patterns
- "Once complete, write JSON to `<absolute_or_run_relative_path>` containing `<required_fields>`."
- "Do not rely on stdout for structured result parsing; only the file at `<path>` is contract output."
- "If generation fails, still write a structured error object to `<path>`."

## How To Verify
1. Run a minimal workflow execution for the edited step.
2. Confirm expected output file exists at exact path.
3. Confirm file content matches required schema/keys.
4. Confirm downstream step can parse and proceed.

## Boundaries
- Never rely on stdout for machine-parsed outputs.
- Keep wording deterministic; avoid ambiguous output instructions.
- Do not change unrelated workflow graph logic unless required for contract alignment.

## Deliverable
Return:
- Prompt/context changes.
- Contract compliance notes.
- Any risks to parseability or output mapping.
