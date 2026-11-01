//! Offline search over the SQLite FTS5 index. "peru 1841" gives one row per locality, nearest first: the exact
//! number where OSM has it, one estimated between the two closest numbers of that street where it doesn't. Without a
//! number: streets, neighbourhoods and places. With nothing found it corrects to the closest word of the index and,
//! last, accepts matching any of the words. Python has the same algorithm; `comparar-nativo` checks they agree.

use rusqlite::{params, Connection, OpenFlags, Row};
use serde_json::{json, Value};
use std::collections::HashSet;
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
    street: Option<String>,
    lat: f64,
    lon: f64,
    number: Option<i64>,
    rank: f64,
}

fn place(r: &Row) -> rusqlite::Result<Place> {
    Ok(Place { name: r.get("nombre")?, label: r.get("etiqueta")?, detail: r.get("detalle")?, street: r.get("calle")?, lat: r.get("lat")?, lon: r.get("lon")?,
               number: r.get("numero")?, rank: r.get("rango").unwrap_or(0.0) })
}

fn km(lat: f64, lon: f64, near: Option<(f64, f64)>) -> f64 {
    near.map_or(0.0, |(la, lo)| (lat - la).hypot((lon - lo) * la.to_radians().cos()) * 111.0)
}

fn out(name: &str, label: &str, detail: Option<&str>, lat: f64, lon: f64) -> Value {
    json!({"nombre": name, "tipo": label, "detalle": detail, "lat": lat, "lon": lon})
}

/// "Barracas, CABA · altura estimada" -> "CABA".
fn region(detail: &str) -> String {
    let zone = detail.split(" · ").next().unwrap_or(detail);
    zone.rsplit(", ").next().unwrap_or(zone).to_string()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = previous + usize::from(ca != *cb);
            previous = row[j + 1];
            row[j + 1] = (row[j + 1] + 1).min(row[j] + 1).min(cost);
        }
    }
    row[b.len()]
}

pub struct Search {
    db: Connection,
}

impl Search {
    pub fn open(path: &str) -> Result<Self, String> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| format!("{path}: {e}"))?;
        Ok(Self { db })
    }

    /// The index's closest word, if this one isn't there even as the start of another.
    fn correct(&self, word: &str) -> rusqlite::Result<String> {
        let len = word.chars().count();
        let upper = |w: &str| format!("{w}\u{ffff}");
        if len < 4 || self.db.prepare_cached("SELECT 1 FROM palabras WHERE palabra >= ? AND palabra < ? LIMIT 1")?.exists(params![word, upper(word)])? {
            return Ok(word.to_string());
        }
        let limit = if len >= 7 { 2 } else { 1 };
        let start: String = word.chars().take(2).collect();
        let mut sql = self.db.prepare_cached("SELECT palabra, veces FROM palabras WHERE palabra >= ? AND palabra < ? ORDER BY palabra")?;
        let rows = sql.query_map(params![start, upper(&start)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut best: Option<((usize, i64), String)> = None;
        for row in rows {
            let (candidate, times) = row?;
            let d = edit_distance(word, &candidate);
            if d <= limit && best.as_ref().is_none_or(|b| (d, -times) < b.0) {
                best = Some(((d, -times), candidate));
            }
        }
        Ok(best.map_or_else(|| word.to_string(), |b| b.1))
    }

    pub fn search(&self, q: &str, near: Option<(f64, f64)>, limit: usize) -> rusqlite::Result<Vec<Value>> {
        let tokens = normalize(q);
        let (numbers, words): (Vec<&str>, Vec<&str>) = tokens.split(' ').filter(|t| !t.is_empty()).partition(|t| t.bytes().all(|b| b.is_ascii_digit()));
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let number = numbers.last().and_then(|n| n.parse::<i64>().ok());
        let words: Vec<String> = words.iter().map(ToString::to_string).collect();
        let mut found = self.attempt(&words, " ", number, near, limit)?;
        let fixed: Vec<String> = words.iter().map(|w| self.correct(w)).collect::<Result<_, _>>()?;
        if found.is_empty() && fixed != words {
            found = self.attempt(&fixed, " ", number, near, limit)?;
        }
        if found.is_empty() && fixed.len() > 1 {
            let long: Vec<String> = fixed.into_iter().filter(|w| w.chars().count() > 2).collect();
            found = self.attempt(&long, " OR ", number, near, limit)?;
        }
        Ok(found)
    }

    fn attempt(&self, words: &[String], join: &str, number: Option<i64>, near: Option<(f64, f64)>, limit: usize) -> rusqlite::Result<Vec<Value>> {
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let m = words.iter().map(|t| format!("\"{t}\"*")).collect::<Vec<_>>().join(join);
        match number {
            Some(n) => {
                let street = words.iter().filter(|w| *w != "av").cloned().collect::<Vec<_>>().join(" ");
                self.addresses(&m, &street, n, near, limit)
            }
            None => self.places(&m, near, limit),
        }
    }

    fn places(&self, m: &str, near: Option<(f64, f64)>, limit: usize) -> rusqlite::Result<Vec<Value>> {
        let sql = "SELECT l.*, bm25(lugares_fts) AS rango FROM lugares_fts JOIN lugares l ON l.id = lugares_fts.rowid \
                   WHERE lugares_fts MATCH ? AND l.tipo != 'direccion' ORDER BY rango LIMIT 3000";
        let mut found: Vec<Place> = self.db.prepare_cached(sql)?.query_map([m], place)?.collect::<Result<_, _>>()?;
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

    /// One row per street and locality: the number where OSM has it, estimated within that locality where not.
    fn addresses(&self, m: &str, wanted: &str, number: i64, near: Option<(f64, f64)>, limit: usize) -> rusqlite::Result<Vec<Value>> {
        let sql = "SELECT l.* FROM lugares_fts JOIN lugares l ON l.id = lugares_fts.rowid \
                   WHERE lugares_fts MATCH ? AND l.tipo = 'direccion' AND l.numero = ? LIMIT 200";
        let mut exact: Vec<Place> = self.db.prepare_cached(sql)?.query_map(params![format!("({m}) AND \"{number}\""), number], place)?.collect::<Result<_, _>>()?;
        exact.sort_by(|a, b| (a.name.contains('('), km(a.lat, a.lon, near)).partial_cmp(&(b.name.contains('('), km(b.lat, b.lon, near))).unwrap_or(std::cmp::Ordering::Equal));
        let mut seen: HashSet<(String, Option<String>)> = HashSet::new();
        let mut res: Vec<(f64, Value)> = Vec::new();
        for p in &exact {
            if seen.insert((p.street.clone().unwrap_or_default(), p.detail.clone())) {
                res.push((km(p.lat, p.lon, near), out(&p.name, &p.label, p.detail.as_deref(), p.lat, p.lon)));
            }
        }
        let sql = "SELECT DISTINCT l.calle, l.nombre FROM lugares_fts JOIN lugares l ON l.id = lugares_fts.rowid \
                   WHERE lugares_fts MATCH ? AND l.tipo = 'calle' LIMIT 60";
        let streets: Vec<(String, String)> = self.db.prepare_cached(sql)?.query_map([m], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        for (street, name) in streets {
            let sql = "SELECT * FROM lugares WHERE calle = ? AND tipo = 'direccion' AND numero BETWEEN ? AND ?";
            let close: Vec<Place> = self.db.prepare_cached(sql)?.query_map(params![street, number - 200, number + 200], place)?.collect::<Result<_, _>>()?;
            let mut zones: Vec<(Option<String>, (i64, i64), Vec<&Place>)> = Vec::new();
            for p in &close {
                let cell = if p.detail.is_some() { (0, 0) } else { ((p.lat / 0.02).round() as i64, (p.lon / 0.02).round() as i64) };
                match zones.iter_mut().find(|z| z.0 == p.detail && z.1 == cell) {
                    Some(z) => z.2.push(p),
                    None => zones.push((p.detail.clone(), cell, vec![p])),
                }
            }
            for (zone, _, rows) in &zones {
                if !seen.insert((street.clone(), zone.clone())) {
                    continue;
                }
                // Same parity, same side of the street, when there is one.
                let same: Vec<&&Place> = rows.iter().filter(|p| p.number.unwrap_or(0).rem_euclid(2) == number.rem_euclid(2)).collect();
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
                let detail = zone.as_ref().map_or("altura estimada".to_string(), |z| format!("{z} · altura estimada"));
                res.push((km(lat, lon, near), out(&format!("{name} {number}"), "Dirección", Some(&detail), lat, lon)));
            }
        }
        res.sort_by(|a, b| a.0.total_cmp(&b.0));
        // The same street in other localities, with no numbers near to estimate: last, and it leads to the street.
        let sql = "SELECT l.* FROM lugares_fts JOIN lugares l ON l.id = lugares_fts.rowid \
                   WHERE lugares_fts MATCH ? AND l.tipo = 'calle' LIMIT 400";
        let mut others: Vec<Place> = self.db.prepare_cached(sql)?.query_map([m], place)?.collect::<Result<_, _>>()?;
        others.sort_by(|a, b| km(a.lat, a.lon, near).total_cmp(&km(b.lat, b.lon, near)));
        let mut regions: HashSet<Option<String>> = res.iter().map(|r| r.1["detalle"].as_str().map(region)).collect();
        for p in &others {
            let street = p.street.clone().unwrap_or_default();
            // One per district: another stretch of the same street in the same district adds nothing.
            let there = p.detail.as_deref().map(region);
            if street.strip_prefix("av ").unwrap_or(&street) == wanted && (there.is_none() || !regions.contains(&there)) {
                regions.insert(there);
                let detail = p.detail.as_ref().map_or("calle, sin esa altura".to_string(), |z| format!("{z} · calle, sin esa altura"));
                res.push((0.0, out(&p.name, "Dirección", Some(&detail), p.lat, p.lon)));
            }
        }
        Ok(res.into_iter().take(limit + 4).map(|r| r.1).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{edit_distance, normalize, region};

    #[test]
    fn counts_the_letters_that_differ() {
        assert_eq!(edit_distance("ingenio", "ingenia"), 1);
        assert_eq!(edit_distance("abasto", "abasto"), 0);
        assert_eq!(edit_distance("casa", "caso"), 1);
        assert_eq!(edit_distance("peru", "pera"), 1);
        assert_eq!(edit_distance("abc", ""), 3);
    }

    #[test]
    fn the_district_is_the_last_part() {
        assert_eq!(region("Barracas, CABA · altura estimada"), "CABA");
        assert_eq!(region("San Justo"), "San Justo");
    }

    #[test]
    fn searches_as_people_write() {
        assert_eq!(normalize("McDonald's"), "mcdonalds");
        assert_eq!(normalize("Avenida Perú"), "av peru");
    }
}
