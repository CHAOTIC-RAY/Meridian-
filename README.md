# Meridian Cinemas

Cinema booking prototype with a 3D seat-view preview, seat selection, five-minute holds, checkout and ticket history.

## Cloudflare deployment (frontend first)

Meridian is configured for Cloudflare Workers with Workers Static Assets. The current deployment deliberately has **no database binding**. It can host the frontend and Worker, but booking API endpoints return HTTP 503 until an external database and booking service are connected.

### Deploy

Requirements: Node.js, npm, and a Cloudflare account.

```sh
npm install
npm run typecheck
npm run dev
npm run deploy
```

Wrangler serves static files from `public/`. Requests under `/api/*` are routed through `src/worker.js`. Check `/api/health` to confirm the Worker is running and see the current storage configuration.

## Database integration (deferred)

No Cloudflare D1 database, migration, or database resource is created by this repository. The next backend step is to choose an external provider (Appwrite or another suitable service), then implement the API against that provider. Keep the frontend API contract stable where possible:

- `GET /api/showtimes/:key/seats?client=`
- `GET /api/showtimes/:key/events`
- `POST /api/holds`
- `POST /api/holds/release`
- `POST /api/bookings`
- `GET /api/bookings?client=`

Do not use the current demo client identifier as authentication. Add proper user authentication, server-side authorization, and verified payment-provider callbacks before handling real bookings or payments.

## Original local Rust backend

The original Axum + SQLite implementation remains in `Cargo.toml` and `src/main.rs` for local/self-hosted development:

```sh
cargo run --release
```

It serves the frontend on `http://localhost:3000`. Set `PORT` or `DB` to override the defaults.
