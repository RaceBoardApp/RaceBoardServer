# GitLab Pipeline Adapter for Raceboard

This adapter tracks GitLab CI/CD pipelines as races in Raceboard, providing real-time visibility of your pipeline status.

## Features

- Tracks pipelines from GitLab.com or self-hosted GitLab instances
- Filters pipelines to show only those triggered by you
- Updates pipeline progress based on job completion
- Supports automatic retry with exponential backoff
- Persists state to avoid duplicate processing

## Prerequisites

1. Raceboard server running on `http://localhost:7777`
2. GitLab personal access token with `read_api` and `read_user` scopes
3. Your GitLab user ID

## Installation

### Build from source

```bash
cargo build --release --bin raceboard-gitlab
```

The binary will be available at `./target/release/raceboard-gitlab`

## Configuration

1. Copy the example configuration:
```bash
cp gitlab_config.toml.example gitlab_config.toml
```

2. Edit `gitlab_config.toml`:

```toml
[gitlab]
url = "https://gitlab.com"  # Or your self-hosted GitLab URL
api_token = "glpat-xxxxxxxxxxxxxxxxxxxx"  # Your personal access token
user_id = 12345  # Your GitLab user ID

[raceboard]
server_url = "http://localhost:7777"

[sync]
interval_seconds = 30
max_pipelines = 20
lookback_hours = 24
```

### Getting your GitLab credentials

1. **Personal Access Token**: 
   - Go to https://gitlab.com/-/profile/personal_access_tokens
   - Create a new token with `read_api` and `read_user` scopes
   - Copy the token value

2. **User ID**:
   - Go to https://gitlab.com/-/profile
   - Your user ID is in the URL or can be found in the page source

## Usage

1. Ensure Raceboard server is running:
```bash
./target/debug/raceboard-server
```

2. Start the GitLab adapter:
```bash
./target/release/raceboard-gitlab
```

The adapter will:
- Poll GitLab every 30 seconds (configurable)
- Track up to 20 most recent pipelines
- Look back 24 hours for pipeline updates
- Create/update races in Raceboard automatically

## Webhook Support

The adapter supports GitLab webhooks for real-time pipeline updates in addition to periodic polling.

### Setting up Webhooks

1. Enable webhooks in your configuration:
```toml
[webhook]
enabled = true
port = 8082
secret = "your-secure-secret-token"
```

2. Configure webhook in GitLab:
   - Go to your project's Settings → Webhooks
   - URL: `http://YOUR_SERVER:8082/webhooks/gitlab`
   - Secret Token: Same as in your config
   - Trigger events: ✓ Pipeline events, ✓ Job events
   - Click "Add webhook"

3. The adapter will:
   - Accept webhook events on port 8082
   - Verify signatures using the secret token
   - Process pipeline updates in real-time
   - Continue periodic polling as backup

### Webhook Security

- Always use a strong secret token in production
- The adapter verifies all incoming webhooks using HMAC-SHA256
- Invalid signatures are rejected with 401 Unauthorized
- For local testing, you can leave the secret empty to disable verification

## How it Works

### Pipeline to Race Mapping

| GitLab Pipeline | Raceboard Race |
|----------------|----------------|
| Pipeline ID | Race ID: `gitlab-{project_id}-{pipeline_id}` |
| Project + Branch | Title: `{project_name} - {branch}` |
| Pipeline Status | State (see mapping below) |
| Pipeline URL | Deeplink |
| Job completion | Progress percentage |

### State Mapping

| GitLab State | Raceboard State |
|-------------|-----------------|
| created, pending, scheduled | queued |
| preparing, running | running |
| success | passed |
| failed | failed |
| canceled, skipped | canceled |
| manual | queued |

### Progress Calculation

Progress is calculated as the percentage of completed jobs:
```
progress = (completed_jobs / total_jobs) * 100
```

Where completed jobs have status: `success`, `skipped`, or `manual`

## Monitoring

The adapter logs important events:
- INFO: Normal operations (pipelines found, races created)
- WARN: Rate limit approaching, missing data
- ERROR: API failures, race creation failures

Enable debug logging with:
```bash
RUST_LOG=debug ./target/release/raceboard-gitlab
```

## State Persistence

The adapter saves its state to `.gitlab_adapter_state.json` to:
- Track the last sync time
- Remember processed pipeline IDs
- Avoid duplicate processing after restart

## Troubleshooting

### No pipelines appearing
- Check your user ID is correct
- Verify the token has proper scopes
- Ensure you have pipelines in the last 24 hours

### Authentication errors
- Regenerate your personal access token
- Ensure the token has `read_api` and `read_user` scopes

### Rate limiting
The adapter respects GitLab's rate limits:
- 2000 requests/hour for authenticated users
- Automatically backs off when rate limited
- Retries with exponential backoff on failures

### Connection errors
- Check your GitLab URL is correct
- For self-hosted instances, ensure the URL is accessible
- The adapter will retry 3 times with exponential backoff

## Development

To modify the adapter:
1. Edit `src/bin/raceboard_gitlab.rs`
2. Rebuild: `cargo build --bin raceboard-gitlab`
3. Test with debug logging: `RUST_LOG=debug cargo run --bin raceboard-gitlab`

## License

Same as Raceboard Server