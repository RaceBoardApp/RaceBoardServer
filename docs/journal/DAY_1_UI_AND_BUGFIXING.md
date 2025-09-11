# Day 1: UI and Core Functionality ✅ COMPLETED

The goal for Day 1 is to implement the "smooth UI for ETA updates" and to polish the existing UI.

## Morning (4 hours): Implement Smooth UI for ETA Updates ✅ COMPLETED

*   **Task:** Implement the "Optimistic Progress v2" feature as described in the `OPTIMISTIC_PROGRESS_V2.md` document.
*   **Details:**
    *   ✅ Update the `RaceViewModel` to track the `lastServerProgress`, `lastServerETA`, and `lastServerAt` for each race.
    *   ✅ Implement the "dual-rail" progress bar in ~~`RaceCardView`~~ `RaceRowView`, with a solid bar for the authoritative progress and a striped overlay for the predicted progress.
    *   ✅ Implement the logic for displaying the "Revised ETA" hint when the ETA increases.

**Implementation Notes:**
- Server-side: Added all required fields (last_progress_update, last_eta_update, eta_source, eta_confidence, update_interval_hint, eta_history)
- UI-side: DualRailProgressView implemented with solid + striped overlay
- ETA revision detection with "Revised ETA" pill (1.2s display)
- Trust windows based on source type
- Visual indicators: ≈ symbol, pulsing/solid status dots

## Afternoon (4 hours): Polish and Bug Fixing ✅ COMPLETED

*   **Task:** Address the most critical bugs and UI/UX issues.
*   **Details:**
    *   ✅ Fix the UI "jumping" issue when a race finishes (solved via non-regression guarantee in dual-rail).
    *   ✅ Implement the hover tint for the race cards (isHovering state in RaceRowView).
    *   ✅ Add the inline "X" for dismissing finished races (onDismiss handler with animation).
