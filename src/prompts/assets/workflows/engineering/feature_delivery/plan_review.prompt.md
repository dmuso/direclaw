Review the implementation plan for correctness, sequencing, and risk coverage.
Approve only if the plan is actionable and complete for delivery.
Decision semantics for this gate:
- `approve` => plan is acceptable; proceed to task decomposition.
- `reject` => plan needs revision; return actionable feedback.
Write outputs exactly to:
decision -> {{workflow.output_paths.decision}}
summary -> {{workflow.output_paths.summary}}
feedback -> {{workflow.output_paths.feedback}}
