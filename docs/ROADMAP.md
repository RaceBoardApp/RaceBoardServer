# 3-Day Implementation Roadmap

This document outlines the plan for the next 3 days of development.

## Day 1: Focus on the Core User Experience ✅ COMPLETED

The goal for Day 1 is to implement the "smooth UI for ETA updates" and to polish the existing UI.

*   **Morning (4 hours): Implement Smooth UI for ETA Updates** ✅ COMPLETED
    *   **Task:** Implement the "Optimistic Progress v2" feature as described in the `OPTIMISTIC_PROGRESS_V2.md` document.
    *   **Details:**
        *   ✅ Update the `RaceViewModel` to track the `lastServerProgress`, `lastServerETA`, and `lastServerAt` for each race.
        *   ✅ Implement the "dual-rail" progress bar in ~~`RaceCardView`~~ `RaceRowView`, with a solid bar for the authoritative progress and a striped overlay for the predicted progress.
        *   ✅ Implement the logic for displaying the "Revised ETA" hint when the ETA increases.

*   **Afternoon (4 hours): Polish and Bug Fixing** ✅ COMPLETED
    *   **Task:** Address the most critical bugs and UI/UX issues.
    *   **Details:**
        *   ✅ Fix the UI "jumping" issue when a race finishes (solved via dual-rail non-regression).
        *   ✅ Implement the hover tint for the race cards (hover state in RaceRowView).
        *   ✅ Add the inline "X" for dismissing finished races (onDismiss handler implemented).

## Day 2: Installation and Cluster Sharing

The goal for Day 2 is to make the application easier to install and to lay the groundwork for the cluster sharing feature.

*   **Morning (4 hours): Decompose and Start the "One-Click Installation"**
    *   **Task:** Decompose the "one-click installation" task and implement the first step.
    *   **Decomposition:**
        1.  **Create a simple installation script:** This script will install the server and the UI, but it will not handle dependencies or updates.
        2.  **Create a more advanced installer:** This installer will handle dependencies and will provide a more user-friendly installation experience.
        3.  **Implement auto-updates:** This will be the final step in creating a true "one-click" installation experience.
    *   **Today's Goal:** Implement **Step 1** by creating a simple installation script.

*   **Afternoon (4 hours): Design the Cluster Sharing Feature**
    *   **Task:** Create a new design document for the cluster sharing feature.
    *   **Details:** The document will describe:
        *   The API for exporting and importing clusters.
        *   The data format for the exported clusters.
        *   The UI for managing the shared clusters.

## Day 3: Documentation and Final Polish

The goal for Day 3 is to write the necessary documentation and to do a final round of testing and bug fixing.

*   **Morning (4 hours): Write Documentation** ✅ PARTIALLY COMPLETED
    *   **Task:** Write the documentation for creating new adapters and for configuring the existing adapters.
    *   **Details:**
        *   ✅ Create a new `ADAPTER_DEVELOPMENT_GUIDE.md` that explains how to create a new adapter.
        *   ⚠️  Create a new `ADAPTER_CONFIGURATION.md` that explains how to configure the existing adapters (Configuration docs exist in individual adapter docs).

*   **Afternoon (4 hours): Final Polish and Release Prep**
    *   **Task:** Do a final round of testing and bug fixing, and prepare for the release.
    *   **Details:**
        *   Perform end-to-end testing of all new features.
        *   Fix any remaining bugs.
        *   Prepare the GitHub repo for the release.
        *   Write a draft of the social media post.
