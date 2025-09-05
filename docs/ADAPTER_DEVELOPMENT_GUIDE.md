# Adapter Development Guide (v2)

Related docs:
- Server Guide: `docs/SERVER_GUIDE.md`
- HTTP API: `api/openapi.yaml`
- Calendar adapter details: `docs/GOOGLE_CALENDAR_ADAPTER.md`

This guide provides everything you need to know to build your own custom adapters for Raceboard.

## 1. Introduction

(No changes)

## 2. Getting Started: Your First Adapter

(No changes)

## 3. The Raceboard REST API

The Raceboard server exposes a simple REST API for adapters. The base URL for the API is `http://localhost:7777`.

### Endpoints

*   `POST /race`: Create a new race.
*   `GET /race/{id}`: Get the details of a single race.
*   `GET /races`: List all active races.
*   `PATCH /race/{id}`: Update an existing race.
*   `DELETE /race/{id}`: Delete a race.
*   `POST /race/{id}/event`: Add an event to a race.
*   `GET /historic/races`: Query historical race data.

### Race States

A race can be in one of the following states:

*   `queued`: The race has been created but has not yet started.
*   `running`: The race is currently in progress.
*   `passed`: The race has completed successfully.
*   `failed`: The race has completed with an error.
*   `canceled`: The race has been canceled by the user.

### Important Fields

*   `started_at`: The time the race started, in ISO 8601 format.
*   `deeplink`: A URL that links directly to the task in the source system.
*   `progress`: An integer from 0 to 100 that represents the progress of the race.

## 4. Optimizing for Better ETA Predictions

(No changes)

## 5. Best Practices

### Race Lifecycle Management

**IMPORTANT: Never delete races!** Races should be preserved as historical data for clustering and ETA predictions.

*   **State Transitions:** A typical race will transition through the following states: `queued` -> `running` -> `passed` or `failed`.
*   **Completed Races:** When a task completes, update the race with the final state (`passed`, `failed`, or `canceled`) instead of deleting it.
*   **Race Deletion:** The `DELETE /race/{id}` endpoint should only be used in exceptional cases (e.g., duplicate races, test data cleanup). Never delete races as part of normal operation.
*   **Historical Data:** Completed races are valuable for:
    - Clustering similar tasks
    - Improving ETA predictions
    - Providing historical analytics
    - Understanding task patterns

### Update Patterns

*   **Long-Running Tasks:** For long-running tasks, provide regular progress updates to keep the user informed.
*   **Progress Update Frequency:** For most tasks, a progress update every 5-10 seconds is sufficient.
*   **Final State Updates:** Always send a final update when a task completes with the appropriate state:
    - `passed`: Task completed successfully
    - `failed`: Task encountered an error
    - `canceled`: Task was canceled by user or system
*   **Event Logging:** Use the event logging endpoint to provide detailed information about the progress of the race.

### Example: Proper Race Lifecycle

```rust
// When a pipeline starts
let race = Race {
    id: format!("gitlab-{}", pipeline.id),
    source: "gitlab",
    state: "running",
    // ... other fields
};
upsert_race(&race).await?;

// During execution (periodic updates)
race.progress = Some(50);
upsert_race(&race).await?;

// When pipeline completes
race.state = match pipeline.status {
    "success" => "passed",
    "failed" => "failed",
    "canceled" => "canceled",
    _ => race.state,
};
race.progress = Some(100);
upsert_race(&race).await?;
// DO NOT delete the race here!
```

## 6. Error Handling

The Raceboard server uses standard HTTP status codes to indicate the success or failure of an API request. If a request fails, the body of the response will contain a JSON object with an `error` field that explains the reason for the failure.

