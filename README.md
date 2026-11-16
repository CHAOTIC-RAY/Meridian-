# Meridian Cinemas

Cinema booking prototype with a 3D seat-view preview, seat selection, five-minute holds, checkout and ticket history.

## Deployment targets

- **Cloudflare Workers + D1 + Workers Static Assets** (recommended hosted deployment)
- **Rust/Axum + SQLite** (local/self-hosted development backend)

The browser API contract is shared: `/api/showtimes/:key/seats`, `/api/showtimes/:key/events`, `/api/holds`, `/api/holds/release`, and `/api/bookings`.

## Cloudflare setup

Requirements: a Cloudflare account, Node.js, and npm.

1. Install dependencies:

   ```sh
   npm install
   ```

2. Create the D1 database:

   ```sh
   npx wrangler d1 create meridian-db
   ```

   Copy the returned database UUID into `wrangler.jsonc`, replacing `REPLACE_WITH_D1_DATABASE_ID`.

3. Apply the schema to the remote database:

   ```sh
   npm run db:migrate:remote
   ```

4. Deploy the Worker and the contents of `public/`:

   ```sh
   npm run deploy
   ```

Wrangler serves static files from `public/` and routes `/api/*` through the Worker. The D1 binding is named `DB`; static assets are available through `ASSETS`.

For local development, run `npm run db:migrate:local` and then `npm run dev`.

## Local Rust backend

The original Axum/SQLite backend remains available:

```sh
cargo run --release
```

It serves the frontend on `http://localhost:3000`. Set `PORT` or `DB` to override the defaults.

## Notes

- D1 is the persistent store for Cloudflare deployments; no SQLite file is written by the Worker.
- Seat holds expire after five minutes and booking totals are calculated server-side.
- The events endpoint emits periodic SSE refresh events so clients re-check seat availability.
- The client identifier in this demo is browser-generated, not an authentication identity. Add real user authentication and payment-provider verification before accepting real payments or personal data.
