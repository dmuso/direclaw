---
name: direclaw-scheduler-cron-management
description: "Use when adding, editing, deleting, or debugging scheduled cron tasks, including tracing schedule -> enqueue -> route -> execute."
---

# DireClaw Scheduler and Cron Management

## Purpose
Safely add, edit, delete, or debug cron tasks while preserving reliable scheduled execution.

## DireClaw Context
Scheduled behavior crosses stages:
- schedule definition and timing evaluation
- enqueue of trigger work
- selector routing
- workflow/function execution
- completion write-back
- timezone interpretation can change expected run times if not explicit

## What To Do
1. Identify requested action:
   - add cron task
   - edit cron task
   - delete cron task
   - debug existing cron task
2. For add/edit/delete actions, apply the smallest targeted scheduler job change via scheduler command paths (`schedule.create`, `schedule.update`, `schedule.delete`, `schedule.pause`, `schedule.resume`, `schedule.run_now`).
3. Normalize time window with explicit timezone and absolute times.
4. Validate schedule definition syntax and effective cadence.
5. Confirm scheduler evaluated and attempted enqueue at expected times.
6. Trace each expected enqueue through:
   - queue creation
   - selector routing
   - workflow run creation
   - step execution/completion
7. Identify first failing stage and classify failure type:
   - missed
   - late
   - duplicate
   - failed execution
8. Propose smallest fix at first failing stage.

## Where To Check
- Scheduler job definitions:
  - `<current_working_directory>/automation/jobs/`
- Scheduler worker state:
  - `<current_working_directory>/automation/scheduler_state.json`
- Scheduler run history:
  - `<current_working_directory>/automation/runs/`
- Scheduler-related logs:
  - `<current_working_directory>/logs/`
- Queue artifacts:
  - `<current_working_directory>/queue/`
- Selector and run artifacts:
  - `<current_working_directory>/orchestrator/select/`
  - `<current_working_directory>/workflows/runs/`

## How To Verify
1. Recompute expected trigger times for the same window/timezone.
2. Confirm job file state in `<current_working_directory>/automation/jobs/` matches intended action.
3. Match each expected trigger to enqueue artifacts.
4. Match enqueue artifacts to scheduler run history and workflow completion records.
5. Re-run a short validation window after applying fix.

## Boundaries
- Fix only the first failing stage unless asked for broader hardening.
- Keep timestamps explicit to avoid timezone confusion.
- Do not guess schedule semantics without checking configured timezone and cron expression.
- Do not modify unrelated workflows, agents, or prompt contracts.

## Deliverable
Return:
- Action summary (`added`, `edited`, `deleted`, or `debugged`).
- Stage-by-stage diagnosis.
- First failing stage.
- Evidence paths with timestamps.
- Minimal change/fix plan.
