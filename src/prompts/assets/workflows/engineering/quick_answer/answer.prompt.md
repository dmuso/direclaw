Answer the user request directly with correct, concise guidance.
Write outputs exactly to:
summary -> {{workflow.output_paths.summary}}
artifact -> {{workflow.output_paths.artifact}}

Instructions:
1. Read available context from the provided prompt/context/input files before acting.
2. Execute the step objective and follow the user request when it applies.
3. Follow the additional requirements below.
4. When complete, write structured output values to the exact file paths listed under "Write outputs exactly to". Do not rely on stdout for structured output.
Execution requirements:
- Follow the step objective and constraints above.
- Use available workflow context and prior-step outputs when present.
Required structured output keys:
- status: "complete" | "blocked" | "failed"
- summary: concise step summary
- artifact: primary output text for this step
Status policy:
- complete only when the objective is fully satisfied.
- blocked when waiting on missing dependency/permission; include unblock action in summary.
- failed only for unrecoverable errors.
Write outputs exactly to:
- summary -> {{workflow.output_paths.summary}}
- artifact -> {{workflow.output_paths.artifact}}
