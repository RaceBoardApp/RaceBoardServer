# Dynamic ETA Updates Proposal

This document proposes a new feature for the Raceboard server that will allow it to dynamically update the ETA for a race when new information becomes available.

## 1. Concept

The current ETA prediction system calculates the ETA for a race when it is first created. However, the ETA for a long-running task can change over time as new information becomes available. This proposal suggests that the server should be able to dynamically update the ETA for a race in response to new information from the adapters.

## 2. Goals

*   **More Accurate ETAs:** The primary goal of this feature is to provide more accurate ETAs for long-running tasks.
*   **More Responsive UI:** A more accurate ETA will lead to a more responsive and useful UI.

# Dynamic ETA Updates Proposal (v2)

This document proposes a new feature for the Raceboard server that will allow it to dynamically update the ETA for a race when new information becomes available.

## 1. Concept

(No changes)

## 2. Goals

(No changes)

## 3. Staged Races

To support dynamic ETA updates, we will introduce the concept of "staged races". A staged race is a race that is composed of multiple stages, each with its own ETA. This will allow us to model the behavior of long-running tasks, such as CI/CD pipelines, more accurately.

## 4. Clustering for Staged Races

The clustering engine will be updated to create separate clusters for each stage of a race. This will be done by adding a `stage` field to the `Race` model and using this field as part of the key for the clusters.

## 5. Data Model Changes

The following changes will be made to the data model:

*   A `stage` field will be added to the `Race` model.
*   A new `RaceStage` model will be created to store the history of each stage of a race. The `RaceStage` model will be stored as an event in the `Race`'s `events` list.

## 6. Implementation Plan

*   **Phase 1: Data Model Changes:** Implement the changes to the data model.
*   **Phase 2: Update Clustering Engine:** Update the clustering engine to support staged races.
*   **Phase 3: Update Prediction Engine:** Update the `PredictionEngine` to expose a new `recalculate_eta` function.
*   **Phase 4: Update API Handlers:** Update the API handlers to call the `recalculate_eta` function when they receive new information from an adapter.
*   **Phase 5: Update UI:** Update the UI to handle the new ETA updates.

