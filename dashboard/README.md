# Sockudo Operator Dashboard

Separate admin UI for managing Sockudo apps, policy, observability, webhooks, and push. Dashboard **users** are stored
in the dashboard database (migrations), not in environment variables.

## Structure

```text
dashboard/
├── api/     Bun + Hono admin API (auth, users, CRUD, metrics proxy)
└── web/     Vue 3 operator UI
```

## Available Features

### Dashboard API (`dashboard/api`)

Bun + Hono service on port **3460** (`/api/v1/*`):

- **Auth** — email/password login, JWT session cookies, logout, `/auth/me`
- **Users** — DB-backed operators with `admin` and `operator` roles; create, update, delete, change password
- **Apps** — list, create, update, delete Sockudo applications; rotate app secrets; manage the
  complete app policy (limits, features, channels, namespaces, history, recovery, and idempotency)
- **Webhooks** — per-app webhook CRUD against the Sockudo app store
- **Ops** — proxy to Sockudo `/stats`, `/usage`, `/metrics`, and health endpoints; the Prometheus
  parser preserves HELP/TYPE metadata, labels, timestamps, and histogram families
- **Push** — allowlisted, signed proxy for provider credentials, devices, subscriptions, publish
  status, and dead letters; reads require a dashboard session and mutations require an admin
- **Bootstrap** — automatic migrations and optional first-admin seed on startup

Supports `mysql`, `pgsql`, and `dynamodb` app managers (not `memory`).

### Dashboard UI (`dashboard/web`)

Vue 3 + Vite operator UI on port **5174**:

- **Login** — session-based sign-in against the dashboard API
- **Apps** — browse and manage applications
- **App detail** — structured General, Limits, Channels, Reliability, and Webhooks editors
- **Metrics** — configurable Prometheus workbench with metric discovery, value/rate/average panels,
  per-app filtering, SVG time-series, scrape intervals, and bounded browser-local history
- **Push manager** — upload APNs, FCM, Web Push, HMS, or WNS credentials; inspect devices and
  subscriptions; publish notifications; inspect status and replay dead letters
- **Users** — admin-only user management

Docker services: `dashboard-api`, `dashboard-web` (see [Docker](#docker) below).

## Prerequisites

1. Sockudo must use a **durable** app manager (not `memory`):

   ```bash
   APP_MANAGER_DRIVER=pgsql   # or mysql, dynamodb
   ```

2. Enable operational endpoints and metrics on Sockudo:

   ```bash
   HTTP_API_USAGE_ENABLED=true
   METRICS_ENABLED=true
   ```

3. For push management, build Sockudo with the `push` feature and configure durable push storage.
   Credential upload also requires `PUSH_CREDENTIAL_ENCRYPTION_KEY`. Provider delivery features
   and credentials are loaded by Sockudo itself; the dashboard does not run delivery workers.

## Quick start

```bash
# 1. Configure environment
cp dashboard/.env.example .env
# Replace the empty DASHBOARD_SESSION_SECRET value with the output of:
openssl rand -base64 32

# 2. Install API, run migrations, seed first admin
cd dashboard/api
bun install
bun run migrate
bun run seed:admin admin@sockudo.local 'change-me-now' 'Admin'

# 3. Start API
bun run dev

# 4. Start UI (separate terminal)
cd dashboard/web && bun install && bun run dev
```

- API: http://localhost:3460
- UI: http://localhost:5174

## Docker

`make up` (default `ENV=dev`) starts Sockudo **and** the operator dashboard:

```bash
make setup    # first time: creates .env
make build    # build images including dashboard
make up       # starts sockudo, redis, mysql, dashboard-api, dashboard-web
```

| Service | URL |
|---------|-----|
| Dashboard UI | http://localhost:5174 |
| Dashboard API | http://localhost:3460 |

Set `DASHBOARD_SESSION_SECRET` and optional `DASHBOARD_SEED_EMAIL` / `DASHBOARD_SEED_PASSWORD` in `.env` before first run.

Or manually with compose:

```bash
# Set a session secret and optional first admin seed in .env
export DASHBOARD_SESSION_SECRET=$(openssl rand -base64 32)
export DASHBOARD_SEED_EMAIL=admin@sockudo.local
export DASHBOARD_SEED_PASSWORD='change-me-now'

docker compose \
  -f docker-compose.yml \
  -f docker-compose.dev.yml \
  -f docker-compose.dashboard.yml \
  up -d --build
```

| Service | URL |
|---------|-----|
| Dashboard UI | http://localhost:5174 |
| Dashboard API | http://localhost:3460 |
| Sockudo | http://localhost:6001 |

Images are built from `dashboard/Dockerfile` (`api` and `web` targets). Migrations and
optional admin seed run automatically on API startup.

**Cargo / Rust:** the dashboard is not part of the Rust workspace; no `Cargo.toml` changes
are required.

## Helm

The `charts/sockudo` chart can deploy the operator dashboard alongside Sockudo.
The dashboard is disabled by default and must be backed by a durable app manager
(`mysql`, `pgsql`/`postgres`, or `dynamodb`).

Create the dashboard session secret outside the chart for production:

```bash
kubectl create secret generic sockudo-dashboard-session \
  --from-literal=dashboard-session-secret="$(openssl rand -base64 32)"
```

Example values:

```yaml
config:
  appManagerDriver: pgsql
  httpApi:
    usageEnabled: true
  metrics:
    enabled: true

database:
  postgres:
    host: postgres.default.svc
    port: 5432
    username: sockudo
    password: ""
    database: sockudo
    tableName: applications
  existingSecret: sockudo-postgres

dashboard:
  enabled: true
  sessionSecret:
    existingSecret: sockudo-dashboard-session
  seedAdmin:
    enabled: true
    existingSecret: sockudo-dashboard-seed
  ingress:
    enabled: true
    hosts:
      - host: sockudo-admin.example.com
        paths:
          - path: /
            pathType: Prefix
    tls:
      - secretName: sockudo-admin-tls
        hosts:
          - sockudo-admin.example.com
```

`dashboard-api` and `dashboard-web` images default to the chart app version and
can be overridden with `dashboard.api.image.*` and `dashboard.web.image.*`.
The web image proxies `/api/*` to the release-scoped dashboard API service at
runtime. If you use `dashboard.databaseDriver=sqlite`, also set
`dashboard.persistence.enabled=true` or provide `dashboard.persistence.existingClaim`;
SQLite mode supports only one dashboard API replica.

## Dashboard database

Dashboard users live in separate tables (`dashboard_users`, `dashboard_migrations`):

| `APP_MANAGER_DRIVER` | Dashboard DB used |
|----------------------|-------------------|
| `pgsql` | Same PostgreSQL database as Sockudo apps |
| `mysql` | Same MySQL database as Sockudo apps |
| `dynamodb` | Local SQLite (`dashboard/api/data/dashboard.sqlite`) |

Override with `DASHBOARD_DATABASE_DRIVER=sqlite|mysql|pgsql`.

Migrations run automatically on API startup. To run manually:

```bash
cd dashboard/api && bun run migrate
```

Seed the first admin (only when no users exist):

```bash
bun run seed:admin <email> <password> [name]
```

## User management API

| Method | Path | Access | Description |
|--------|------|--------|-------------|
| POST | `/api/v1/auth/login` | Public | Login with email/password |
| GET | `/api/v1/auth/me` | Auth | Current user |
| GET | `/api/v1/users` | Admin | List users |
| POST | `/api/v1/users` | Admin | Create user |
| GET | `/api/v1/users/:id` | Admin or self | Get user |
| PUT | `/api/v1/users/:id` | Admin or self | Update user |
| DELETE | `/api/v1/users/:id` | Admin | Delete user |
| POST | `/api/v1/users/:id/change-password` | Admin or self | Change password |

Roles: `admin` (full access including user management), `operator` (apps/webhooks/metrics).

## Environment variables

| Variable | Purpose |
|----------|---------|
| `APP_MANAGER_DRIVER` | Sockudo app store: `mysql`, `pgsql`, `dynamodb` |
| `DASHBOARD_DATABASE_DRIVER` | Override dashboard DB driver |
| `DASHBOARD_SESSION_SECRET` | Required JWT session signing secret (at least 32 bytes; no fallback) |
| `DASHBOARD_TRUST_PROXY` | Trust the first `X-Forwarded-For` address for login limits; enable only behind a proxy that replaces the header |
| `DASHBOARD_SQLITE_PATH` | SQLite path when using sqlite driver |
| `DATABASE_*` | Shared DB credentials (mysql/pgsql) |
| `SOCKUDO_HTTP_URL` | Sockudo main port for `/stats` |
| `SOCKUDO_METRICS_URL` | Metrics port |

**Do not** put operator credentials in `.env` — use `bun run seed:admin` or the Users UI.

## Cache note

Sockudo caches apps in memory. After dashboard app/webhook changes, nodes may take up to
the app-manager database cache TTL (300 seconds by default) to pick up updates. Secret rotations,
disables, and deletes have the same propagation window on already-running nodes.

## Metrics model

The dashboard API scrapes the single endpoint configured by `SOCKUDO_METRICS_URL`. Panels and a
bounded six-hour/180-point history are stored in the operator's browser, not in the dashboard
database. This makes the built-in workbench useful without another dependency, but it is not a
cluster-wide time-series database. Use Prometheus plus Grafana for durable retention, alerting,
multi-node aggregation, and long-range queries.

## Push credential model

- Provider material travels from an admin browser through the authenticated dashboard API to
  Sockudo and is not stored by the dashboard.
- Sockudo encrypts credential material and list responses expose only provider, credential ID, and
  version. The UI therefore reports a credential as **stored**, never as active or healthy.
- FCM and APNs credential selection is currently boot-time and app-specific; a worker restart may
  be required after rotation.
- Stored Web Push, HMS, and WNS credentials are not yet consumed by all monolith worker paths;
  consult the server push configuration for the provider-specific runtime requirements.
- The native API currently has no credential delete or provider-readiness endpoint.

## Security

- Use strong passwords (min 8 chars) for all dashboard users.
- Set a random, unique `DASHBOARD_SESSION_SECRET` in every environment. The API fails at startup if it is missing, shorter than 32 bytes, or a documented placeholder. Generate one with `openssl rand -base64 32`.
- Login attempts are limited in bounded in-process windows by both source IP and normalized account identity. Successful authentication clears the account window; source-IP attempts remain counted. Multi-replica deployments should also enforce a shared limit at the ingress.
- Existing sessions are rejected immediately when their user is deleted or disabled, and password changes invalidate previously issued sessions. Current database roles are applied on every protected request, so demotions take effect immediately.
- Deploying this hardening change invalidates older dashboard cookies that lack the credential-version, issuer, and audience claims; operators must sign in again once.
- Put the dashboard behind TLS and restrict network access.
- Push credential uploads and notification publishing are admin-only. Signed upstream URLs,
  app secrets, provider private keys, service-account JSON, and credential bodies are never logged
  or returned by the dashboard proxy.
