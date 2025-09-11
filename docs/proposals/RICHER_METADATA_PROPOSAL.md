# Richer Metadata Proposal

This document proposes a plan for collecting richer metadata from the Raceboard adapters to improve the accuracy of the ETA prediction system.

## 1. Introduction

The accuracy of the ETA prediction system is highly dependent on the quality of the data that it is trained on. By collecting richer and more detailed metadata from the adapters, we can significantly improve the accuracy of the predictions.

## 2. Goals

*   **More Accurate ETA Predictions:** The primary goal of collecting richer metadata is to improve the accuracy of the ETA predictions.
*   **Better Insights:** Richer metadata will also provide better insights into the development process, which can be used to identify bottlenecks and to improve efficiency.

# Richer Metadata Proposal (v2)

This document proposes a plan for collecting richer metadata from the Raceboard adapters to improve the accuracy of the ETA prediction system.

## 1. Introduction

(No changes)

## 2. Goals

(No changes)

## 3. Proposed Metadata by Adapter

### 3.1. `raceboard-cmd`

**Current Metadata:**

*   `command`
*   `working_dir`
*   `exit_code`
*   `duration_sec`

**Proposed Additional Metadata:**

*   `user`

### 3.2. `raceboard-claude-adapter`

**Current Metadata:**

*   `prompt`
*   `prompt_hash`
*   `editor`
*   `task_type`
*   `estimated_complexity`

**Proposed Additional Metadata:**

*   `model`
*   `prompt_length`
*   `response_length`
*   `turn_count`
*   `tools_used`

### 3.3. `raceboard-codex-watch`

**Current Metadata:**

*   `prompt`
*   `log_file`
*   `trigger`

**Proposed Additional Metadata:**

*   `file_path`
*   `language`
*   `generated_code_length`
*   `function_call_count`

### 3.4. `raceboard-gemini-watch`

**Current Metadata:**

*   `cwd`
*   `telemetry_file`
*   `prompt_len`
*   `duration_ms`
*   `tool_calls`

**Proposed Additional Metadata:**

*   `model`
*   `response_length`
*   `turn_count`

### 3.5. `gitlab-adapter`

**Current Metadata:**

*   `project_name`
*   `branch`
*   `commit_sha`
*   `pipeline_url`

**Proposed Additional Metadata:**

*   `merge_request_iid`
*   `pipeline_triggerer`
*   `job_count`
*   `stage_count`

### 3.6. `google-calendar-adapter`

**Current Metadata:**

*   `calendar_id`
*   `event_id`
*   `recurring_id`
*   `end_time`
*   `timezone`
*   `meeting_link`
*   `location`
*   `attendees`
*   `organizer`
*   `visibility`
*   `reminders_overridden`
*   `event_hash`
*   `event_category`

**Proposed Additional Metadata:**

*   `is_recurring`

## 4. Implementation Plan

(No changes)

