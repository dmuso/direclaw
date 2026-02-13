Review this codebase for improvements. Particularly focus on:

- Broken, duplicated or unused code paths
- Tests that use fake servers or responses rather than testing the real code
- Opportunities for writing tests for important/critical areas of code
- Missing API contract tests for endpoints, conditions or behaviours
- Skipped tests that need to be implemented
- Placeholder code that has not been completed
- Functions that were never actually implemented
- TODOs that have not been completed
- Error conditions that are not checked appropriately
- Security vulnerabilities like unsantised user inputs
- The use of panics or fatal errors in server code that would crash the server when the error could be recovered
- File structures not following Domain Driven Design principles
- Opportunities for types to be used to enforce consistency and lower risk of bugs
- Variables, modules, function and test names are named well and reflect their intended business value/logic

DireClaw architecture related focus areas:
- Ensure that messaging and agent input/ouput is managed by files.
- Ensure that all agent capabilities are done via `claude` or `codex` CLI tools.

Write a report document as a list of actionable tasks to the `docs/review` folder with a file name like `review-report-YYYYMMDDHHMMSS.md`. Get the latest timestamp using the `date` CLI command. Include in your report specific file and line number references with the context and clear paths to fix the issue. This report is feedback for a junior engineer, so it needs to be clear, concise and actionable with the right information for a fix.
