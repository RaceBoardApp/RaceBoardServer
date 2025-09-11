# Dual‑Rail Progress Verification (Condensed)

Status: Implemented and active in the primary UI flow (RaceboardView → RaceRowView). Dual‑rail shows server progress as solid and optimistic overlay as striped.

Highlights
- Non‑regression enforced; overlay capped; “≈” shown for approximate ETAs.
- Revision pills appear when ETA increases beyond threshold.
- Preferences control overlay enablement and caps.

Notes
- A legacy single‑rail path exists in a compact/secondary view; main list uses dual‑rail.

Impact
- Eliminates UI jumps on server updates and improves perceived responsiveness while data is fresh.
