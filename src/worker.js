const HOLD_SECS = 300;
const MOVIES = new Set(["dune", "neon", "moon", "bloom", "peak"]);
const COLS = 14;

const json = (data, status = 200, extra = {}) =>
  new Response(JSON.stringify(data), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store", ...extra },
  });
const error = (status, message) => json({ error: message }, status);
const now = () => Math.floor(Date.now() / 1000);
const id = (prefix) => prefix + crypto.randomUUID().replaceAll("-", "");

function validKey(key) {
  const [movie, date, time, format] = key.split("|");
  return key.length < 160 && MOVIES.has(movie) && !!date && !!time && !!format;
}
function validSeat(seat) {
  if (typeof seat !== "string" || !/^[A-I](?:[1-9]|1[0-4])$/.test(seat)) return false;
  return true;
}
function price(seat) { return seat.charCodeAt(0) >= 72 ? 16 : 12.5; }
function clientValue(value) {
  return typeof value === "string" && value.length > 0 && value.length <= 40;
}
async function prepareShowtime(db, key) {
  // Seed marker and generated sold seats in one atomic batch. If two first-time
  // requests race, INSERT OR IGNORE keeps the deterministic seed idempotent.
  const marker = await db.prepare("SELECT 1 FROM seeded WHERE showtime = ?").bind(key).first();
  if (!marker) {
    let seed = 2166136261;
    for (const byte of new TextEncoder().encode(key)) seed = Math.imul(seed ^ byte, 16777619) >>> 0;
    const statements = [
      db.prepare("INSERT OR IGNORE INTO seeded (showtime) VALUES (?)").bind(key),
    ];
    for (let row = 0; row < 9; row++) {
      for (let col = 1; col <= COLS; col++) {
        seed = (seed + 0x6D2B79F5) >>> 0;
        let t = seed;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        const random = ((t ^ (t >>> 14)) >>> 0) / 4294967296;
        if (random < (row > 5 ? 0.3 : 0.38)) {
          statements.push(db.prepare(
            "INSERT OR IGNORE INTO seats (showtime, seat, kind, owner) VALUES (?, ?, 'sold', '')"
          ).bind(key, String.fromCharCode(65 + row) + col));
        }
      }
    }
    await db.batch(statements);
  }
  await db.prepare("DELETE FROM seats WHERE kind='held' AND expires <= ?").bind(now()).run();
}

async function handleApi(request, env, url) {
  const { pathname, searchParams } = url;
  const db = env.DB;

  if (request.method === "GET" && pathname === "/api/health") {
    return json({ ok: true, service: "meridian-api", database: "connected" });
  }

  const seatsMatch = pathname.match(/^\/api\/showtimes\/([^/]+)\/seats$/);
  if (request.method === "GET" && seatsMatch) {
    const key = decodeURIComponent(seatsMatch[1]);
    if (!validKey(key)) return error(404, "Unknown showtime");
    await prepareShowtime(db, key);
    const client = searchParams.get("client") || "";
    const { results = [] } = await db.prepare(
      "SELECT seat, kind, owner FROM seats WHERE showtime = ?"
    ).bind(key).all();
    const taken = [], mine = [];
    for (const row of results) {
      if (row.kind === "held" && row.owner === client) mine.push(row.seat);
      else taken.push(row.seat);
    }
    return json({ taken, mine });
  }

  const eventsMatch = pathname.match(/^\/api\/showtimes\/([^/]+)\/events$/);
  if (request.method === "GET" && eventsMatch) {
    const key = decodeURIComponent(eventsMatch[1]);
    if (!validKey(key)) return error(404, "Unknown showtime");
    const stream = new ReadableStream({
      start(controller) {
        const send = () => {
          try { controller.enqueue(new TextEncoder().encode("data: update\n\n")); }
          catch { clearInterval(timer); }
        };
        send();
        const timer = setInterval(send, 12000);
        request.signal.addEventListener("abort", () => {
          clearInterval(timer);
          try { controller.close(); } catch {}
        }, { once: true });
      },
      cancel() {},
    });
    return new Response(stream, {
      headers: {
        "content-type": "text/event-stream; charset=utf-8",
        "cache-control": "no-cache, no-transform",
        "connection": "keep-alive",
        "x-accel-buffering": "no",
      },
    });
  }

  if (request.method === "POST" && pathname === "/api/holds") {
    let body;
    try { body = await request.json(); } catch { return error(400, "Invalid JSON"); }
    const { showtime, seats, client } = body || {};
    if (!validKey(showtime) || !Array.isArray(seats) || seats.length < 1 || seats.length > 6 ||
        !clientValue(client) || !seats.every(validSeat) || new Set(seats).size !== seats.length) {
      return error(400, "Invalid request");
    }
    await prepareShowtime(db, showtime);
    const hold = id("h"), expires = now() + HOLD_SECS;
    const statements = [
      db.prepare("DELETE FROM seats WHERE showtime = ? AND kind = 'held' AND owner = ?").bind(showtime, client),
      ...seats.map(seat => db.prepare(
        "INSERT INTO seats (showtime, seat, kind, owner, hold, expires) VALUES (?, ?, 'held', ?, ?, ?)"
      ).bind(showtime, seat, client, hold, expires)),
    ];
    try {
      await db.batch(statements);
    } catch {
      return error(409, "One or more seats were just taken. Please refresh and try again.");
    }
    return json({ id: hold, expires: expires * 1000 });
  }

  if (request.method === "POST" && pathname === "/api/holds/release") {
    let body;
    try { body = await request.json(); } catch { return error(400, "Invalid JSON"); }
    const { hold, client } = body || {};
    if (!clientValue(client) || typeof hold !== "string" || hold.length > 80) return error(400, "Invalid request");
    await db.prepare("DELETE FROM seats WHERE hold = ? AND owner = ? AND kind = 'held'")
      .bind(hold, client).run();
    return json({ ok: true });
  }

  if (request.method === "POST" && pathname === "/api/bookings") {
    let body;
    try { body = await request.json(); } catch { return error(400, "Invalid JSON"); }
    const { hold, client, tt, popcorn = 0, drink = 0 } = body || {};
    const factor = ({ adult: 1, student: 0.8, child: 0.6 })[tt];
    if (!clientValue(client) || typeof hold !== "string" || !factor ||
        !Number.isInteger(popcorn) || !Number.isInteger(drink) || popcorn < 0 || drink < 0 ||
        popcorn > 10 || drink > 10) return error(400, "Invalid request");

    const held = await db.prepare(
      "SELECT showtime, seat FROM seats WHERE hold = ? AND owner = ? AND kind = 'held' AND expires > ? ORDER BY seat"
    ).bind(hold, client, now()).all();
    const rows = held.results || [];
    if (!rows.length) return error(410, "Your hold expired. Please pick seats again.");

    const total = Math.round((rows.reduce((sum, row) => sum + price(row.seat) * factor, 0) +
      popcorn * 5 + drink * 3.5) * 100) / 100;
    const reference = "MRD-" + crypto.randomUUID().replaceAll("-", "").slice(0, 5).toUpperCase();
    const showtime = rows[0].showtime;
    const seatList = rows.map(row => row.seat);
    const result = await db.batch([
      db.prepare(
        "INSERT INTO bookings (ref, showtime, client, seats, tt, total, created) " +
        "SELECT ?, ?, ?, ?, ?, ?, ? WHERE EXISTS (" +
        "SELECT 1 FROM seats WHERE hold = ? AND owner = ? AND kind = 'held' AND expires > ?)"
      ).bind(reference, showtime, client, seatList.join(","), tt, total, now(), hold, client, now()),
      db.prepare(
        "UPDATE seats SET kind = 'sold', hold = NULL, expires = NULL WHERE hold = ? AND owner = ? AND kind = 'held' AND expires > ?"
      ).bind(hold, client, now()),
    ]);
    if (!result[0]?.meta?.changes) return error(410, "Your hold expired. Please pick seats again.");
    return json({ ref: reference, showtime, seats: seatList, tt, total }, 201);
  }

  if (request.method === "GET" && pathname === "/api/bookings") {
    const client = searchParams.get("client") || "";
    if (!clientValue(client)) return json([]);
    const { results = [] } = await db.prepare(
      "SELECT ref, showtime, seats, tt, total FROM bookings WHERE client = ? ORDER BY id DESC"
    ).bind(client).all();
    return json(results.map(row => ({
      ref: row.ref, showtime: row.showtime, seats: row.seats.split(","), tt: row.tt, total: row.total,
    })));
  }

  return error(404, "Not found");
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname.startsWith("/api/")) {
      if (request.method === "OPTIONS") {
        return new Response(null, { headers: {
          "access-control-allow-origin": url.origin,
          "access-control-allow-methods": "GET, POST, OPTIONS",
          "access-control-allow-headers": "content-type",
          "access-control-max-age": "86400",
        }});
      }
      try { return await handleApi(request, env, url); }
      catch (cause) {
        console.error("Meridian API error", cause);
        return error(500, "An internal server error occurred");
      }
    }
    return env.ASSETS.fetch(request);
  },
};
