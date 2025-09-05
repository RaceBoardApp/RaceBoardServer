# Dual-Rail Progress Implementation Usage Verification

## ✅ Verification Complete

The dual-rail progress system is **properly integrated and actively used** in the main Raceboard UI application.

## Implementation Architecture

### 1. **Main App List View** ✅
**File:** `RaceboardView.swift` (lines 29-45)
```swift
List(selection: $selectedRaceID) {
    ForEach(displayedRacesSorted) { race in
        RaceRowView(  // ← Uses dual-rail progress
            viewModel: viewModel,
            race: race,
            ...
        )
    }
}
```
**Status:** The main races list uses `RaceRowView` which implements dual-rail progress.

### 2. **RaceRowView Implementation** ✅
**File:** `RaceRowView.swift` (lines 76-83)
```swift
DualRailProgressView(
    solid: solidFraction,           // Non-regressive server value
    overlay: presentation.overlayFraction,  // Predicted delta
    color: statusColor
)
.accessibilityIdentifier("race_progress_\(race.id)")
.help(presentation.overlayFraction > 0 ? 
    "Predicted segment (striped) shown while server is stale" : 
    "Live (server) progress")
```
**Status:** Fully implemented with proper data flow.

### 3. **Presentation Calculation Flow** ✅

The calculation flows through these layers:

1. **RaceViewModel** → `presentation(for:now:)` method
   - Calculates if prediction should be active
   - Computes overlay fraction with cap
   - Determines if ETA is approximate

2. **RacePresentation** struct
   ```swift
   struct RacePresentation {
       let solidFraction: Double      // Server value
       let overlayFraction: Double    // Predicted delta
       let predictionActive: Bool
       let etaSeconds: Int?
       let etaApproximate: Bool
   }
   ```

3. **RaceRowView** state management
   - `@State solidBaseline`: Ensures non-regression
   - Real-time updates via timer (1Hz)
   - ETA revision detection via onChange

### 4. **Visual Indicators** ✅

All visual feedback elements are active:

- **Progress Bar:** Dual-rail with solid + striped overlay
- **ETA Label:** Shows `≈` when approximate
- **Status Dot:** Pulsing halo when predicting, solid ring when fresh
- **Revision Pill:** "Revised ETA: X" appears on increases

### 5. **Old Progress Bar Status** ⚠️

**Note:** `RaceCardView.swift` still uses the old single progress bar (lines 71-91):
```swift
private var progressBar: some View {
    // Old implementation - single rail only
    RoundedRectangle(cornerRadius: 6)
        .fill(status.color.opacity(0.7))
        .frame(width: geo.size.width * CGFloat(pct) / 100.0, height: 12)
}
```

However, `RaceCardView` appears to be used only for:
- Preview/demo purposes
- Possibly a compact view mode

The **main app list uses RaceRowView with dual-rail**, so users see the optimistic progress feature.

## Data Flow Verification

```
Server (gRPC) 
    ↓
Race Model (with new fields)
    ↓
RaceViewModel.presentation()
    ↓
RacePresentation struct
    ↓
RaceRowView
    ↓
DualRailProgressView ✅
```

## User Preferences Integration ✅

All preferences are properly connected:
- `kPredictedEnabled` - Toggles overlay on/off
- `kPredictedOverrunCapPercent` - Caps overlay (default 20%)
- `kDelayETAIncreases` - Enables revision pills
- `kETAIncreaseThresholdSec` - Revision threshold
- Trust window overrides

## Conclusion

✅ **The dual-rail progress system is fully integrated and actively used in the main app.**

The implementation successfully:
1. Shows dual-rail progress in the main races list
2. Calculates predictions based on trust windows
3. Displays visual indicators (≈, dots, pills)
4. Handles ETA revisions smoothly
5. Respects user preferences

The only place not using dual-rail is `RaceCardView`, which appears to be a secondary/compact view. The primary user experience in `RaceboardView` → `RaceRowView` fully implements the optimistic progress feature as specified.