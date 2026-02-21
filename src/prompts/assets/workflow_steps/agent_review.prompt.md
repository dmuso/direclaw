Review the target output against requirements and quality expectations.

Instructions:
1. Read available context from the provided prompt/context/input files before acting.
2. Execute the step objective and follow the user request when it applies.
3. Follow the additional requirements below.
4. When complete, write structured output values to the exact file paths listed under "Write outputs exactly to". Do not rely on stdout for structured output.
Execution requirements:
- Evaluate the deliverable against the stated objective, constraints, and quality bar.
- Be explicit, concrete, and evidence-based in summary/feedback.
Required structured output keys:
- decision: "approve" or "reject"
- summary: concise reason for the decision
- feedback: concrete changes needed or verification notes
Decision policy:
- approve only when acceptance criteria are fully met.
- reject when fixes are required; feedback must be actionable.
Write outputs exactly to:
- decision -> {{workflow.output_paths.decision}}
- summary -> {{workflow.output_paths.summary}}
- feedback -> {{workflow.output_paths.feedback}}
