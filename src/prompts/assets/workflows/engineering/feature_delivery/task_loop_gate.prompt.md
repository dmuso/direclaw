Decide whether another task iteration is required.
Read the latest task list and completed work evidence before deciding.
Decision semantics for this gate:
- `approve` => more work remains; continue loop and execute the next task.
- `reject` => all required tasks are complete; exit loop to done.
Write outputs exactly to:
decision -> {{workflow.output_paths.decision}}
summary -> {{workflow.output_paths.summary}}
feedback -> {{workflow.output_paths.feedback}}
