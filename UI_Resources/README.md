# UI Resources Package

This directory contains all the binaries, configurations, and scripts needed for the Raceboard UI to implement one-click installation as specified in Day 2 requirements.

## Contents

### `/bin/` - Embedded Binaries
Pre-built, release-mode binaries ready for embedding in the UI app:
- `raceboard-server` - Main API server (HTTP/gRPC)
- `raceboard-gitlab` - GitLab CI/CD pipeline tracker
- `raceboard-calendar` - Google Calendar integration
- `raceboard-codex-watch` - AI coding session monitor
- `raceboard-claude` - Claude AI interaction tracker
- `raceboard-gemini-watch` - Gemini AI session monitor

All binaries are:
- Built for macOS ARM64 (Apple Silicon)
- Optimized release builds
- Ready for code signing and notarization

### `/configs/` - Configuration Templates
Template configuration files for UI to customize:
- `server_config_template.toml` - Server configuration with localhost binding
- `gitlab_config_template.toml` - GitLab adapter template
- `calendar_config_template.toml` - Google Calendar adapter template

### `/scripts/` - Helper Scripts
Utility scripts for the UI helper app:
- `check_ports.sh` - Finds available ports for server
- `health_check.sh` - Monitors server and adapter health

### `manifest.json`
Complete manifest describing:
- Server and adapter specifications
- Required environment variables
- CLI flags for each binary
- Authentication requirements
- App Group directory structure
- Required entitlements

## Integration Steps for UI

1. **Embed in Login Item Helper**
   - Copy entire `bin/` directory to `LoginItemHelper.app/Contents/Resources/bin/`
   - Ensure binaries are included in code signing

2. **First Launch Setup**
   - Use `check_ports.sh` to find available ports
   - Copy config templates to App Group container
   - Customize configs with actual ports and paths
   - Store secrets in Keychain, not config files

3. **Process Management**
   - Launch server first with environment variables:
     ```bash
     RACEBOARD_SERVER__HTTP_PORT=<port> \
     RACEBOARD_SERVER__GRPC_PORT=<port> \
     RACEBOARD_PERSISTENCE__PATH=<group>/eta_history.db \
     ./raceboard-server
     ```
   - Launch selected adapters with `--server` and `--config` flags
   - Use `health_check.sh` for monitoring

4. **App Group Container Structure**
   ```
   ~/Library/Group Containers/<teamid>.raceboard/
   ├── config.toml          # Server config
   ├── eta_history.db/      # Sled database
   ├── logs/                # Process logs
   ├── configs/             # Adapter configs
   └── bookmarks/           # Security-scoped bookmarks
   ```

5. **Required Entitlements**
   See `manifest.json` for complete list of required entitlements for both main app and login helper.

## Testing

Before App Store submission:
1. Verify all binaries run sandboxed
2. Test with restricted file system access
3. Confirm localhost-only binding
4. Validate OAuth flows
5. Test process supervision and restart

## Updates

When updating:
1. Replace binaries in this directory
2. Update version in manifest.json
3. Test migration with existing App Group data
4. Ensure backwards compatibility