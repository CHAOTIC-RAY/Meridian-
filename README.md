# Meridian Cinemas

Cinema booking app: browse films, pick seats, preview the view from your seat in 3D, check out and hold seats for 5 minutes.
Rust (axum + SQLite) backend, single-file Three.js frontend.

## Run
    cargo run --release        # http://localhost:3000  (run from the project root)

Data is stored in `meridian.db` (override with `DB=path`, port with `PORT=`). Keep `Cargo.lock`: it pins dependency versions.

## Layout
- `src/main.rs` – REST API + server-sent events + static hosting
- `public/index.html` – frontend (talks to the API)
- `preview/index.html` – same frontend with an in-browser mock API, open it directly with no server

## API
| Endpoint | Purpose |
|---|---|
| GET  /api/showtimes/:key/seats?client= | taken seats (sold + held by others) and your own holds |
| GET  /api/showtimes/:key/events | SSE ping when seats at that showtime change |
| POST /api/holds {showtime, seats[1-6], client} | all-or-nothing 5-minute hold; 409 if a seat is gone |
| POST /api/holds/release {hold, client} | release a hold |
| POST /api/bookings {hold, client, tt, popcorn, drink} | confirm a hold; price computed server-side; 410 if expired |
| GET  /api/bookings?client= | that client's tickets |

`:key` is `movie|date|time|format`, e.g. `dune|Sun 15 Nov|19:45|IMAX` (URL-encoded).
