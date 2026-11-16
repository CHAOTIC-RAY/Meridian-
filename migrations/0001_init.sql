CREATE TABLE IF NOT EXISTS seeded (
  showtime TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS seats (
  showtime TEXT NOT NULL,
  seat TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('sold', 'held')),
  owner TEXT NOT NULL DEFAULT '',
  hold TEXT,
  expires INTEGER,
  PRIMARY KEY (showtime, seat)
);

CREATE INDEX IF NOT EXISTS idx_seats_hold ON seats (hold, owner, kind);
CREATE INDEX IF NOT EXISTS idx_seats_expiry ON seats (kind, expires);

CREATE TABLE IF NOT EXISTS bookings (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ref TEXT NOT NULL UNIQUE,
  showtime TEXT NOT NULL,
  client TEXT NOT NULL,
  seats TEXT NOT NULL,
  tt TEXT NOT NULL,
  total REAL NOT NULL,
  created INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_bookings_client ON bookings (client, id DESC);
