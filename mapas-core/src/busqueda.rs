//! Offline search over the SQLite FTS5 index: exact address, an estimated one between the two nearest
//! numbers of the street, or streets, neighbourhoods and places, nearest first.

use rusqlite::{params, Connection, OpenFlags, Row};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

pub fn normalize(text: &str) -> String {
    // No apostrophes: McDonald's is searched as mcdonalds.
    let plain: String = text.to_lowercase().nfkd().filter(|c| !is_combining_mark(*c) && !"'’`´".contains(*c)).collect();
    let words: String = plain.chars().map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == 'ñ' { c } else { ' ' }).collect();
    let short = |w| match w {
        "avenida" | "avda" => "av",
        "doctor" => "dr",
        "general" => "gral",
        "presidente" => "pres",
        "teniente" => "tte",
        "coronel" => "cnel",
        "ingeniero" => "ing",
        "santa" => "sta",
        other => other,
    };
    words.split_whitespace().map(short).collect::<Vec<_>>().join(" ")
}

struct Place {
    name: String,
    label: String,
    detail: Option<String>,
    lat: f64,
    lon: f64,
    number: Option<i64>,
    rank: f64,
}

fn place(r: &Row) -> rusqlite::Result<Place> {
    Ok(Place { name: r.get("nombre")?, label: r.get("etiqueta")?, detail: r.get("detalle")?, lat: r.get("lat")?, lon: r.get("lon")?,
               number: r.get("numero")?, rank: r.get("rango").unwrap_or(0.0) })
}

fn km(lat: f64, lon: f64, near: Option<(f64, f64)>) -> f64 {
    near.map_or(0.0, |(la, lo)| (lat - la).hypot((lon - lo) * la.to_radians().cos()) * 111.0)
}

fn out(name: &str, label: &str, detail: Option<&str>, lat: f64, lon: f64) -> Value {
    json!({"nombre": name, "tipo": label, "detalle": detail, "lat": lat, "lon": lon})
}

pub struct Search {
    db: Connection,
}

impl Search {
    pub fn open(path: &str) -> Result<Self, String> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| format!("{path}: {e}"))?;
        Ok(Self { db })
    }

    pub fn search(&self, q: &str, near: Option<(f64, f64)>, limit: usize) -> rusqlite::Result<Vec<Value>> {
        let tokens = normalize(q);
        let (numbers, words): (Vec<&str>, Vec<&str>) = tokens.split(' ').filter(|t| !t.is_empty()).partition(|t| t.bytes().all(|b| b.is_ascii_digit()));
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let m = words.iter().map(|t| format!("\"{t}\"*")).collect::<Vec<_>>().join(" ");
        if let Some(number) = numbers.last().and_then(|n| n.parse::<i64>().ok()) {
            let sql = "SELECT l.* FROM lugares_fts JOIN lugares l ON l.id = lugares_fts.rowid \
                       WHERE lugares_fts MATCH ? AND l.tipo = 'direccion' AND l.numero = ? LIMIT 40";
            let mut exact: Vec<Place> = self.db.prepare(sql)?.query_map(params![format!("{m} \"{number}\""), number], place)?.collect::<Result<_, _>>()?;
            if !exact.is_empty() {
                exact.sort_by(|a, b| km(a.lat, a.lon, near).total_cmp(&km(b.lat, b.lon, near)));
                return Ok(exact.iter().take(limit).map(|p| out(&p.name, &p.label, p.detail.as_deref(), p.lat, p.lon)).collect());
            }
            return self.estimate(&m, number, near, limit);
        }
        let sql = "SELECT l.*, bm25(lugares_fts) AS rango FROM lugares_fts JOIN lugares l ON l.id = lugares_fts.rowid \
                   WHERE lugares_fts MATCH ? AND l.tipo != 'direccion' ORDER BY rango LIMIT 3000";
        let mut found: Vec<Place> = self.db.prepare(sql)?.query_map([&m], place)?.collect::<Result<_, _>>()?;
        found.sort_by(|a, b| (a.rank + 0.4 * km(a.lat, a.lon, near)).total_cmp(&(b.rank + 0.4 * km(b.lat, b.lon, near))));
        let mut seen = HashSet::new();
        let mut res = Vec::new();
        for p in found {
            if seen.insert((p.name.clone(), (p.lat * 100.0).round() as i64, (p.lon * 100.0).round() as i64)) {
                res.push(out(&p.name, &p.label, p.detail.as_deref(), p.lat, p.lon));
            }
            if res.len() == limit {
                break;
            }
        }
        Ok(res)
    }

    /// The same street name can be in several districts: estimates within each zone of ~2 km.
    fn estimate(&self, m: &str, number: i64, near: Option<(f64, f64)>, limit: usize) -> rusqlite::Result<Vec<Value>> {
        let sql = "SELECT DISTINCT l.calle, l.nombre FROM lugares_fts JOIN lugares l ON l.id = lugares_fts.rowid \
                   WHERE lugares_fts MATCH ? AND l.tipo = 'calle' LIMIT 60";
        let streets: Vec<(String, String)> = self.db.prepare(sql)?.query_map([m], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        let mut res: Vec<(f64, Value)> = Vec::new();
        for (street, name) in streets {
            let sql = "SELECT * FROM lugares WHERE calle = ? AND tipo = 'direccion' AND numero BETWEEN ? AND ?";
            let close: Vec<Place> = self.db.prepare_cached(sql)?.query_map(params![street, number - 200, number + 200], place)?.collect::<Result<_, _>>()?;
            let mut zones: HashMap<(i64, i64), Vec<&Place>> = HashMap::new();
            for p in &close {
                zones.entry(((p.lat / 0.02).round() as i64, (p.lon / 0.02).round() as i64)).or_default().push(p);
            }
            for rows in zones.values() {
                // Same parity, same side of the street, when there is one.
                let same: Vec<&&Place> = rows.iter().filter(|p| p.number.unwrap_or(0) % 2 == number % 2).collect();
                let pool: Vec<&&Place> = if same.is_empty() { rows.iter().collect() } else { same };
                let n = |p: &&&Place| p.number.unwrap_or(0);
                let below = pool.iter().filter(|p| n(p) < number).max_by_key(|p| n(p));
                let above = pool.iter().filter(|p| n(p) > number).min_by_key(|p| n(p));
                let (lat, lon) = match (below, above) {
                    (Some(a), Some(b)) => {
                        let f = (number - n(a)) as f64 / (n(b) - n(a)) as f64;
                        (a.lat + f * (b.lat - a.lat), a.lon + f * (b.lon - a.lon))
                    }
                    (Some(p), None) | (None, Some(p)) => (p.lat, p.lon),
                    (None, None) => continue,
                };
                res.push((km(lat, lon, near), out(&format!("{name} {number}"), "Dirección (aprox.)", None, lat, lon)));
            }
        }
        res.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(res.into_iter().take(limit).map(|r| r.1).collect())
    }
}
