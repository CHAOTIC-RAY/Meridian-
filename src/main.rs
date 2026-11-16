use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, Stream, StreamExt};
use tower_http::services::ServeDir;

const MOVIES: [&str; 5] = ["dune", "neon", "moon", "bloom", "peak"];
const COLS: usize = 14;
const HOLD_SECS: i64 = 300;

struct App {
    db: Mutex<Connection>,
    tx: broadcast::Sender<String>, // showtime key -> "something changed" (fan-out to SSE clients)
}
type St = State<Arc<App>>;
type Res<T> = Result<T, (StatusCode, Json<Value>)>;

fn er<T>(c: StatusCode, m: &str) -> Res<T> { Err((c, Json(json!({ "error": m })))) }
fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64 }
fn nanos() -> u128 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() }

/// A showtime key looks like "dune|Sun 15 Nov|19:45|IMAX".
fn valid_key(k: &str) -> bool { k.len() < 80 && MOVIES.iter().any(|m| k.starts_with(&format!("{m}|"))) }
fn valid_seat(s: &str) -> bool {
    let b = s.as_bytes();
    (2..=3).contains(&s.len()) && (b'A'..=b'I').contains(&b[0])
        && s[1..].parse::<usize>().map(|n| (1..=COLS).contains(&n)).unwrap_or(false)
}
fn seat_price(s: &str) -> f64 { if s.as_bytes()[0] >= b'H' { 16.0 } else { 12.5 } }

fn mulberry(s: &mut u32) -> f64 {
    *s = s.wrapping_add(0x6D2B79F5);
    let mut t = (*s ^ (*s >> 15)).wrapping_mul(1 | *s);
    t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
    ((t ^ (t >> 14)) as f64) / 4294967296.0
}

/// Seed "already sold" seats the first time a showtime is touched, then drop expired holds.
fn prep(c: &Connection, key: &str) {
    let seeded = c.query_row("SELECT 1 FROM seeded WHERE k=?1", [key], |_| Ok(())).is_ok();
    if !seeded {
        let mut seed = key.bytes().fold(2166136261u32, |a, b| (a ^ b as u32).wrapping_mul(16777619));
        for (i, row) in "ABCDEFGHI".chars().enumerate() {
            for col in 1..=COLS {
                if mulberry(&mut seed) < if i > 5 { 0.3 } else { 0.38 } {
                    c.execute("INSERT OR IGNORE INTO seats(showtime,seat,kind,owner) VALUES(?1,?2,'sold','')",
                        params![key, format!("{row}{col}")]).unwrap();
                }
            }
        }
        c.execute("INSERT INTO seeded(k) VALUES(?1)", [key]).unwrap();
    }
    c.execute("DELETE FROM seats WHERE kind='held' AND expires<?1", [now()]).unwrap();
}

#[derive(Deserialize)] struct Cl { client: Option<String> }

async fn seats(State(a): St, Path(key): Path<String>, Query(q): Query<Cl>) -> Res<Json<Value>> {
    if !valid_key(&key) { return er(StatusCode::NOT_FOUND, "Unknown showtime"); }
    let c = a.db.lock().unwrap();
    prep(&c, &key);
    let me = q.client.unwrap_or_default();
    let mut st = c.prepare("SELECT seat,kind,owner FROM seats WHERE showtime=?1").unwrap();
    let rows: Vec<(String, String, String)> = st
        .query_map([&key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().filter_map(|r| r.ok()).collect();
    let (mut taken, mut mine) = (vec![], vec![]);
    for (s, k, o) in rows { if k == "held" && o == me { mine.push(s) } else { taken.push(s) } }
    Ok(Json(json!({ "taken": taken, "mine": mine })))
}

#[derive(Deserialize)] struct HoldReq { showtime: String, seats: Vec<String>, client: String }

async fn hold(State(a): St, Json(b): Json<HoldReq>) -> Res<Json<Value>> {
    if !valid_key(&b.showtime) || b.seats.is_empty() || b.seats.len() > 6 || b.client.is_empty() || b.client.len() > 40
        || !b.seats.iter().all(|s| valid_seat(s)) {
        return er(StatusCode::BAD_REQUEST, "Invalid request");
    }
    let mut c = a.db.lock().unwrap();
    prep(&c, &b.showtime);
    let tx = c.transaction().unwrap(); // all-or-nothing: nobody can slip into the middle of a group
    tx.execute("DELETE FROM seats WHERE showtime=?1 AND kind='held' AND owner=?2", params![b.showtime, b.client]).unwrap();
    for s in &b.seats {
        let n: i64 = tx.query_row("SELECT COUNT(*) FROM seats WHERE showtime=?1 AND seat=?2", params![b.showtime, s], |r| r.get(0)).unwrap();
        if n > 0 { return er(StatusCode::CONFLICT, &format!("Seat {s} was just taken")); }
    }
    let (id, exp) = (format!("h{:x}", nanos()), now() + HOLD_SECS);
    for s in &b.seats {
        tx.execute("INSERT INTO seats(showtime,seat,kind,owner,hold,expires) VALUES(?1,?2,'held',?3,?4,?5)",
            params![b.showtime, s, b.client, id, exp]).unwrap();
    }
    tx.commit().unwrap();
    let _ = a.tx.send(b.showtime.clone());
    Ok(Json(json!({ "id": id, "expires": exp * 1000 })))
}

#[derive(Deserialize)] struct RelReq { hold: String, client: String }

async fn release(State(a): St, Json(b): Json<RelReq>) -> Res<Json<Value>> {
    let c = a.db.lock().unwrap();
    let key: Option<String> = c.query_row("SELECT showtime FROM seats WHERE hold=?1 AND owner=?2 AND kind='held' LIMIT 1",
        params![b.hold, b.client], |r| r.get(0)).ok();
    c.execute("DELETE FROM seats WHERE hold=?1 AND owner=?2 AND kind='held'", params![b.hold, b.client]).unwrap();
    if let Some(k) = key { let _ = a.tx.send(k); }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct BookReq { hold: String, client: String, tt: String, #[serde(default)] popcorn: u32, #[serde(default)] drink: u32 }

async fn book(State(a): St, Json(b): Json<BookReq>) -> Res<(StatusCode, Json<Value>)> {
    let factor = match b.tt.as_str() { "adult" => 1.0, "student" => 0.8, "child" => 0.6, _ => return er(StatusCode::BAD_REQUEST, "Bad ticket type") };
    if b.popcorn > 10 || b.drink > 10 { return er(StatusCode::BAD_REQUEST, "Too many snacks"); }
    let mut c = a.db.lock().unwrap();
    c.execute("DELETE FROM seats WHERE kind='held' AND expires<?1", [now()]).unwrap();
    let rows: Vec<(String, String)> = {
        let mut st = c.prepare("SELECT showtime,seat FROM seats WHERE hold=?1 AND owner=?2 AND kind='held'").unwrap();
        let v = st.query_map(params![b.hold, b.client], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(|r| r.ok()).collect();
        v
    };
    if rows.is_empty() { return er(StatusCode::GONE, "Your hold expired. Please pick seats again."); }
    // price is always computed on the server, never trusted from the client
    let sum: f64 = rows.iter().map(|(_, s)| seat_price(s) * factor).sum::<f64>() + b.popcorn as f64 * 5.0 + b.drink as f64 * 3.5;
    let total = (sum * 100.0).round() / 100.0;
    let reference = format!("MRD-{:05X}", (nanos() % 0xFFFFF) as u32);
    let (key, seat_list) = (rows[0].0.clone(), rows.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>());
    let tx = c.transaction().unwrap();
    tx.execute("UPDATE seats SET kind='sold', hold=NULL, expires=NULL WHERE hold=?1 AND owner=?2", params![b.hold, b.client]).unwrap();
    tx.execute("INSERT INTO bookings(ref,showtime,client,seats,tt,total,created) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![reference, key, b.client, seat_list.join(","), b.tt, total, now()]).unwrap();
    tx.commit().unwrap();
    let _ = a.tx.send(key.clone());
    Ok((StatusCode::CREATED, Json(json!({ "ref": reference, "showtime": key, "seats": seat_list, "tt": b.tt, "total": total }))))
}

async fn bookings(State(a): St, Query(q): Query<Cl>) -> Json<Value> {
    let c = a.db.lock().unwrap();
    let mut st = c.prepare("SELECT ref,showtime,seats,tt,total FROM bookings WHERE client=?1 ORDER BY id DESC").unwrap();
    let list: Vec<Value> = st.query_map([q.client.unwrap_or_default()], |r| {
        let seats: String = r.get(2)?;
        Ok(json!({ "ref": r.get::<_, String>(0)?, "showtime": r.get::<_, String>(1)?,
                   "seats": seats.split(',').collect::<Vec<_>>(), "tt": r.get::<_, String>(3)?, "total": r.get::<_, f64>(4)? }))
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(json!(list))
}

/// Server-sent events: every client watching a showtime is nudged when its seats change.
async fn events(State(a): St, Path(key): Path<String>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let s = BroadcastStream::new(a.tx.subscribe()).filter_map(move |m| match m {
        Ok(k) if k == key => Some(Ok(Event::default().data("update"))),
        _ => None,
    });
    Sse::new(s).keep_alive(KeepAlive::default())
}

#[tokio::main]
async fn main() {
    let conn = Connection::open(std::env::var("DB").unwrap_or_else(|_| "meridian.db".into())).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS seeded(k TEXT PRIMARY KEY);
         CREATE TABLE IF NOT EXISTS seats(showtime TEXT, seat TEXT, kind TEXT, owner TEXT DEFAULT '', hold TEXT, expires INTEGER, PRIMARY KEY(showtime,seat));
         CREATE TABLE IF NOT EXISTS bookings(id INTEGER PRIMARY KEY, ref TEXT, showtime TEXT, client TEXT, seats TEXT, tt TEXT, total REAL, created INTEGER);",
    ).unwrap();
    let app = Arc::new(App { db: Mutex::new(conn), tx: broadcast::channel(256).0 });
    let router = Router::new()
        .route("/api/showtimes/:key/seats", get(seats))
        .route("/api/showtimes/:key/events", get(events))
        .route("/api/holds", post(hold))
        .route("/api/holds/release", post(release))
        .route("/api/bookings", post(book).get(bookings))
        .fallback_service(ServeDir::new("public"))
        .with_state(app);
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".into());
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await.unwrap();
    println!("Meridian running → http://localhost:{port}");
    axum::serve(listener, router).await.unwrap();
}
