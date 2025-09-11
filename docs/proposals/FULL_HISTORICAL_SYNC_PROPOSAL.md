# Full Historical Sync Proposal

This document proposes a new feature for the Raceboard adapters that will allow them to perform a full historical sync of all pipelines from a third-party service like GitLab.

## 1. Concept

The current adapters only fetch the most recent pipelines from the third-party APIs. This is efficient, but it means that the ETA prediction system does not have a complete historical record to learn from. This proposal suggests adding a new feature that will allow the adapters to perform a one-time, full historical sync of all pipelines.

## 2. Goals

*   **More Accurate ETA Predictions:** The primary goal of this feature is to improve the accuracy of the ETA predictions by providing the clustering engine with a complete historical record of all pipelines.
*   **Complete Historical Record:** A full historical sync will also provide a complete historical record of all pipelines, which can be useful for auditing and analysis.

## 3. Proposed Design

### 3.1. One-Time, User-Triggered Sync

The full historical sync will be a one-time operation that is triggered by the user. This will prevent the adapter from putting an unnecessary load on the third-party APIs.

### 3.2. Background Processing

The sync will be performed in a background process to avoid blocking the main thread. The UI will provide feedback on the progress of the sync.

### 3.3. Resumable Sync

The sync will be designed to be resumable in case of failure. The adapter will keep track of the last successfully synced pipeline and will resume the sync from that point.

## 4. Implementation Plan

*   **Phase 1: Add a new `sync` command to the adapters:** This command will trigger the full historical sync.
*   **Phase 2: Implement the background processing logic:** Implement the logic for performing the sync in a background process.
*   **Phase 3: Implement the resumable sync logic:** Implement the logic for resuming the sync in case of failure.
