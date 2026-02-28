Break the approved plan into an ordered, dependency-aware task list.
Output `task_list` as strict JSON with shape:
{"tasks":[{"task_id":"t-1","title":"...","goal":"...","instructions":"...","acceptance_criteria":["..."],"depends_on":["..."],"status":"todo"}]}
Rules:
- `task_id` values must be unique.
- `status` must be `todo` for all newly created tasks.
- `depends_on` must only reference earlier or existing task ids.
Write outputs exactly to:
summary -> {{workflow.output_paths.summary}}
task_list -> {{workflow.output_paths.task_list}}
