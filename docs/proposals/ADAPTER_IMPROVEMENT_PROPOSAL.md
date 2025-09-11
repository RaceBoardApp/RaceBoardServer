# Adapter Improvement Proposal

This document proposes a series of improvements to the Raceboard adapter ecosystem to improve its robustness, maintainability, and quality.

## 1. Problem Statement

The current implementation of the adapters (`claude`, `codex`, `gemini`) has three main issues:

1.  **Code Duplication:** There is a significant amount of code duplication between the adapters. The data models (`Race`, `RaceUpdate`, `Event`) and the logic for interacting with the Raceboard server are redefined in each adapter. This makes the adapters difficult to maintain and evolve.
2.  **Lack of Resilience:** The adapters are not resilient to temporary network issues. If the Raceboard server is unavailable, the adapters will fail without retrying.
3.  **No Tests:** The `raceboard-gemini-watch` and `raceboard-claude-adapter` do not have any unit or integration tests. This makes it difficult to verify their correctness and prevent regressions.

## 2. Proposed Solution

To address these issues, I propose the following three improvements:

### 2.1. Create a `raceboard-client` Library Crate

I propose creating a new `raceboard-client` library crate to house all of the shared code between the adapters. This crate would include:

*   **Data Models:** The `Race`, `RaceUpdate`, and `Event` structs would be defined in this crate and used by all adapters.
*   **`RaceboardClient`:** A new `RaceboardClient` struct would be created to encapsulate the logic for interacting with the Raceboard server's REST API. This client would provide a simple and consistent interface for creating, updating, and completing races.

### 2.2. Implement a Retry Mechanism

The `RaceboardClient` would have a built-in retry mechanism with exponential backoff for all API calls. This would make the adapters more resilient to temporary network issues and improve the overall reliability of the system.

### 2.3. Add Unit and Integration Tests

I propose adding a comprehensive suite of unit and integration tests for all adapters. This would include:

*   **Unit Tests:** Unit tests for the core logic of each adapter, such as parsing log files and handling telemetry data.
*   **Integration Tests:** Integration tests that verify that each adapter can successfully communicate with the Raceboard server. The existing test in `raceboard-codex-watch` can be used as a template for these tests.

## 3. Benefits

The proposed improvements would provide the following benefits:

*   **Reduced Code Duplication:** A shared client library would significantly reduce code duplication, making the adapters easier to maintain and evolve.
*   **Improved Resilience:** A built-in retry mechanism would make the adapters more resilient to temporary network issues.
*   **Higher Quality:** A comprehensive suite of tests would improve the quality and reliability of the adapters and prevent regressions.

## 4. Implementation Plan

I propose implementing these improvements in the following order:

1.  **Create the `raceboard-client` library crate:** This is the first and most important step, as it will provide the foundation for the other improvements.
2.  **Refactor the adapters to use the `raceboard-client`:** Once the client library is created, I will refactor the existing adapters to use it.
3.  **Add tests to the adapters:** After refactoring the adapters, I will add unit and integration tests to ensure their correctness.
