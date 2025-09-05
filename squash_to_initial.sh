#!/bin/bash

# Script to squash all commits into a single initial commit
# This preserves all your current files and creates a clean history

echo "=== Squashing all commits to single initial commit ==="
echo "This will create a backup branch before making changes"
echo ""

# Create a backup branch
echo "Creating backup branch..."
git branch backup-before-squash

# Get the current branch name
CURRENT_BRANCH=$(git branch --show-current)

# Create a new orphan branch (no history)
echo "Creating new clean branch..."
git checkout --orphan temp-initial

# Add all files
echo "Adding all files..."
git add -A

# Create the initial commit with comprehensive message
echo "Creating initial commit..."
git commit -m "Initial commit: Raceboard Server

Raceboard is a local-first productivity tool for tracking long-running tasks (races) 
like CI pipelines, builds, deployments, and other time-consuming processes.

Features:
- Real-time race tracking via REST API and gRPC streaming
- ML-based ETA prediction using DBSCAN clustering with HNSW optimization
- Optimistic Progress v2 with dual-rail visualization
- Multiple adapters (GitLab CI, Google Calendar, Claude AI, Codex)
- Historical data persistence with sled database
- Phased rollout system for gradual deployment
- Trust window-based prediction activation
- Non-regressive progress guarantees

Tech Stack:
- Rust server with Actix-web and Tonic gRPC
- SwiftUI macOS client application
- Machine learning for ETA predictions
- Event-driven architecture with real-time updates

Documentation:
- Comprehensive adapter development guides
- Server configuration documentation
- API specifications and integration guides
- Optimistic progress implementation details

License: MIT"

# Delete the old branch and rename the new one
echo "Replacing old branch..."
git branch -D $CURRENT_BRANCH
git branch -m $CURRENT_BRANCH

# Force push would be needed if already pushed to remote
echo ""
echo "=== SUCCESS ==="
echo "All commits have been squashed into a single initial commit."
echo "Your backup is saved in branch: backup-before-squash"
echo ""
echo "To push to GitHub (this will overwrite remote history):"
echo "  git push origin $CURRENT_BRANCH --force"
echo ""
echo "To restore if something went wrong:"
echo "  git checkout backup-before-squash"
echo ""