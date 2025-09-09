# GitLab Pipeline Adapter

## Overview
The GitLab adapter tracks CI/CD pipelines as races in Raceboard, providing real-time visibility into pipeline status and progress. It now includes automatic discovery of contributed projects in addition to membership projects.

## Key Features

### Pipeline Tracking
- Each GitLab pipeline becomes a race in Raceboard
- Real-time status updates (queued, running, passed, failed, canceled)
- Progress calculation based on job completion
- Direct links to GitLab pipeline pages

### Project Discovery
- **Membership projects**: Where user is member/owner
- **Contributed projects**: Where user has commits/MRs (automatic discovery)
- **Configured projects**: Explicitly specified project IDs
- Intelligent deduplication when projects appear in multiple sources

### Smart Filtering
- Only tracks pipelines from last 24 hours
- Excludes archived projects automatically
- 365-day activity cutoff for contributed projects
- Filters by user ID to show only relevant pipelines

## Configuration

```toml
[gitlab]
url = "https://gitlab.com"              # or self-hosted URL
api_token = "glpat-xxx"                 # Personal access token (read_api scope)
user_id = 12345                         # Your GitLab user ID
project_ids = [123, 456]                # Additional projects (optional)

[gitlab.discovery]
contributed_max_pages = 20              # Max API pages (default: 20)
per_page = 100                          # Results per page (default: 100)

[raceboard]
server_url = "http://localhost:7777"

[sync]
interval_seconds = 30
max_pipelines = 100
lookback_hours = 24

[webhook]
enabled = false                         # Optional webhook support
port = 8082
secret = "your-webhook-secret"
```

## Data Mapping

### Race Structure
- **ID**: `gitlab-{project_id}-{pipeline_id}`
- **Title**: `{project_name} - {branch}` or `{project_name} - Pipeline #{id}`
- **Progress**: `(completed_jobs / total_jobs) * 100`
- **Deeplink**: Direct URL to GitLab pipeline

### State Mapping
| GitLab State | Raceboard State |
|---|---|
| created, pending, scheduled | queued |
| preparing, running | running |
| success | passed |
| failed | failed |
| canceled, skipped | canceled |

## API Integration

### Authentication
- Uses Personal Access Token with `read_api` scope
- Token included in `Private-Token` header
- Supports GitLab.com and self-hosted instances

### Endpoints Used
- `/api/v4/user` - Verify token and get username
- `/api/v4/users/{id}/projects` - Get membership projects
- `/api/v4/users/{username}/contributed_projects` - Get contributed projects (if available)
- `/api/v4/projects/{id}/pipelines` - Get pipeline status
- `/api/v4/projects/{id}/pipelines/{id}/jobs` - Get job details

### Rate Limiting
- Respects GitLab's rate limits (2000 requests/hour)
- Exponential backoff on 429 responses
- Honors `Retry-After` header

## Monitoring

### Health Endpoint
`http://127.0.0.1:8081/health` provides:
- Adapter health status
- Last sync timestamp
- Metrics (API calls, races created/updated, errors)
- Project discovery statistics

### Metrics
- `membership_projects`: Projects where user is member
- `contributed_projects`: Projects where user contributed
- `duplicate_projects`: Projects in both categories
- `contributed_api_supported`: API availability status

## Error Handling

- **Network failures**: 3 retries with exponential backoff
- **Invalid token**: Logs error and stops
- **Missing data**: Uses sensible defaults
- **Rate limits**: Automatic backoff and retry

## Deployment

1. Place binary in: `/Users/user/Documents/Raceboard UI/Resources/raceboard-gitlab`
2. Create configuration file (see above)
3. Run: `raceboard-gitlab --config gitlab_config.toml`
4. Verify health at: `http://127.0.0.1:8081/health`

## Backward Compatibility

- Gracefully falls back to membership-only on older GitLab versions
- No breaking changes to existing configurations
- Existing project_ids configuration still supported