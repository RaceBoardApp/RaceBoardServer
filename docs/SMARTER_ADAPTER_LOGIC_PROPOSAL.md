# Smarter Adapter Logic Proposal

This document proposes a new design for the Raceboard adapters that will make them more intelligent and responsive.

## 1. Concept

The current adapters are relatively simple. They create a race when a task starts and then update it with progress information until the task is complete. This proposal suggests making the adapters smarter by giving them a better understanding of the state of the task they are tracking.

## 2. Goals

*   **More Accurate State Tracking:** The adapters will be able to more accurately track the state of the task they are monitoring.
*   **More Responsive:** The adapters will be more responsive to changes in the state of the task.
*   **Finish on User Input:** The adapters will be able to finish a race if the task they are monitoring is waiting for user input.

## 3. Proposed Design

### 3.1. State Machines

Each adapter will use a state machine to model the behavior of the task it is tracking. This will allow the adapter to have a much more nuanced understanding of the state of the task. For example, instead of just having a single `running` state, the adapter could have a `compiling` state, a `testing` state, and a `deploying` state.

### 3.2. Finish on User Input

The adapters will be able to detect when the task they are monitoring is waiting for user input. When this happens, the adapter will finish the race with a new `waiting_for_input` state. This will allow the user to see at a glance which tasks are blocked on their input.

## 4. Implementation Plan

*   **Phase 1: State Machine Library:** Create a new library that provides a simple and easy-to-use state machine implementation.
*   **Phase 2: Update Adapters:** Update the existing adapters to use the new state machine library.
*   **Phase 3: Finish on User Input:** Implement the logic for detecting when a task is waiting for user input and for finishing the race with the new `waiting_for_input` state.
