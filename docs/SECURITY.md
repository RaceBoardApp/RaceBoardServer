# Security Posture (2025-09-11)

This project currently targets a local-only deployment model where all components (server and adapters) run on the same machine and communicate over localhost (127.0.0.1). Under this assumption, the server does not implement authentication/authorization on its HTTP or gRPC endpoints.

Summary:
- Assumption: single-host, trusted environment (localhost only).
- State: no built-in authn/authz on REST or gRPC.
- Risk: if the server is exposed to an untrusted network, unauthenticated writes and admin operations become possible.

If you need to expose the server beyond localhost:
- Keep server bindings on 127.0.0.1 and front it with a reverse proxy (Nginx/Caddy/Traefik) that:
  - Terminates TLS.
  - Enforces authentication (Basic, OIDC) and IP allow-lists.
  - Applies rate limits to admin endpoints.
- Alternatively, add mTLS between trusted nodes or use signed bearer tokens for adapters.
- Restrict access to admin and diagnostic endpoints (`/admin/*`, `/metrics/*`).

Future options (not implemented):
- Optional feature-flagged token auth for adapters.
- mTLS for adapter <-> server communications in multi-host deployments.
- Configurable auth on admin endpoints (role-based access).

This document complements:
- docs/ARCHITECTURE_REVIEW.md (Security Posture section)
- docs/SERVER_GUIDE.md (Security & Local Deployment)
