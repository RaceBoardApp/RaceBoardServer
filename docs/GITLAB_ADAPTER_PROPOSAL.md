# GitLab Pipeline Adapter Proposal (Final)

This document proposes a new adapter for Raceboard that will track the progress of GitLab CI/CD pipelines.

## 1. Concept

The GitLab adapter will treat each pipeline as a race. This will allow you to see the real-time status of all of your pipelines in the Raceboard UI.

*   **Pipeline Start:** When a pipeline starts, a new race will be created with a `running` status.
*   **Pipeline Progress:** The progress of the race will be updated as the pipeline progresses through its stages.
*   **Pipeline Completion:** When the pipeline finishes, the race will be marked as `passed` or `failed`.

## 2. Key Features

*   **Track Your Own Pipelines:** The adapter will be configured with your GitLab user ID and will only create races for pipelines that you have triggered.
*   **Support for Self-Hosted GitLab:** The adapter will be configurable with the URL of your self-hosted GitLab instance.

## 3. GitLab API Interaction

*   **Endpoint:** The adapter will use the `/api/v4/projects/:id/pipelines` endpoint to get the latest status of all pipelines.
*   **Authentication:** The adapter will use a personal access token and will include it in the `Private-Token` header of all API requests.
*   **API Scopes:** The adapter will require the `read_api` and `read_user` API scopes.
*   **Pagination:** The adapter will handle the pagination of the GitLab API, with a page size of 100 and a maximum of 10 pages.
*   **Rate Limiting:** The adapter will respect GitLab's rate limit of 2000 requests per hour for authenticated users.

## 4. Data Model and State Mapping

### 4.1. Data Model

*   **Race ID:** `gitlab-{project_id}-{pipeline_id}`
*   **Title:** `"{project_name} - {branch}"` or `"{project_name} - Pipeline #{pipeline_id}"`
*   **Metadata:**
    *   `project_name`
    *   `branch`
    *   `commit_sha` (short)
    *   `merge_request_iid`
    *   `pipeline_url`
*   **Deeplink:** The `web_url` of the pipeline.

### 4.2. State Mapping

| GitLab State | Raceboard State |
|---|---|
| `created`, `waiting_for_resource`, `pending`, `scheduled` | `queued` |
| `preparing`, `running` | `running` |
| `success` | `passed` |
| `failed` | `failed` |
| `canceled`, `skipped` | `canceled` |
| `manual` | `queued` (with a note in the metadata) |

## 5. Implementation Details

*   **Progress Calculation:** The progress of a pipeline will be calculated as `(completed_jobs / total_jobs) * 100`, where `completed_jobs` are jobs with a status of `success`, `skipped`, or `manual`.
*   **Pipeline Filtering:** The adapter will filter pipelines based on the user's ID.
*   **Update vs. Create Logic:** The adapter will create a new race if the pipeline status is `created` or `pending`, and will update the race otherwise.
*   **Handling Missing Data:**
    *   Missing `branch` -> "unknown"
    *   Missing `commit_sha` -> ""
    *   Missing `merge_request_iid` -> null
    *   Missing `progress` -> 0
*   **Connection Management:** The adapter will use a retry mechanism with exponential backoff (3 retries, with a 2-second initial backoff) for API calls.
*   **State Persistence:** The adapter will store the `last_sync` time and the IDs of the last seen pipelines to avoid duplicates.
*   **Race Lifecycle:** The adapter will only track pipelines that have been updated in the last 24 hours and will delete a race if the corresponding pipeline is deleted in GitLab.

## 6. Webhook Strategy

The adapter will use a hybrid approach of webhooks and periodic polling to ensure that the data is up-to-date.

*   **Webhook Events:** The adapter will subscribe to the `pipeline` and `job` webhook events.
*   **Webhook Security:** The adapter will use a secret token to verify the authenticity of the webhooks.
*   **Periodic Reconciliation:** The adapter will also periodically poll the GitLab API to reconcile any missed webhooks.

## 7. Monitoring and Observability

*   **Health Checks:** The adapter will expose a `/health` endpoint that can be used to monitor its health.
*   **Metrics:** The adapter will collect metrics on the number of API calls, race updates, and errors.
*   **Logging:** The adapter will have a detailed logging strategy to help with debugging.

## 8. Configuration

```toml
[gitlab]
url = "https://gitlab.mycompany.com"
api_token = "your-personal-access-token"
user_id = 12345

[raceboard]
server_url = "http://localhost:7777"

[sync]
interval_seconds = 30
max_pipelines = 20
lookback_hours = 24
```

## 9. Error Handling

The adapter will be designed to be robust and resilient to errors. The following error handling strategies will be implemented:

*   **API Rate Limiting:** The adapter will respect the `Retry-After` header in the GitLab API response. If it receives a `429 Too Many Requests` response, it will wait for the specified amount of time before retrying.
*   **Network Failures:** The adapter will use an exponential backoff strategy with jitter for retrying network failures.
*   **Invalid/Expired Tokens:** If the adapter receives a `401 Unauthorized` response, it will log an error and stop trying to make requests. It will also send a notification to the user (if configured) to inform them that their token is invalid.
*   **Partial Pipeline Data:** The adapter will be designed to be resilient to partial pipeline data. It will only update the fields that are present in the API response and will not fail if some fields are missing.