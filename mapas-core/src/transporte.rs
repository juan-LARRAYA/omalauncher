//! Public transport: RAPTOR over buses and subway (by frequency: half a headway of waiting) and trains (by
//! timetable: the next one). Walking is a straight line with a 30 % detour, at 1.2 m/s.

use crate::bin_file::BinFile;
use crate::ruteo::dist_m;
use serde_json::{json, Value};
use std::collections::HashMap;

const WALK: f64 = 1.2 / 1.3; // m/s in a straight line
const ACCESS_M: f64 = 800.0;
const CELL: f64 = 0.004; // degrees, ~400 m
const ROUNDS: usize = 4; // up to 3 changes
const MAX_WAIT: i32 = 2 * 3600;

#[derive(Clone, Copy)]
enum From {
    Start(f64),
    Ride { route: usize, on: usize, off: usize, wait: i32 },
    Walk { stop: usize, secs: i32 },
}

pub struct Network {
    f: BinFile,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

impl Network {
    pub fn open(path: &str) -> Result<Self, String> {
        let f = BinFile::open(path)?;
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        {
            let (lat, lon) = (f.slice::<f64>("p_lat"), f.slice::<f64>("p_lon"));
            for i in 0..lat.len() {
                grid.entry(((lat[i] / CELL).floor() as i32, (lon[i] / CELL).floor() as i32)).or_default().push(i as u32);
            }
        }
        Ok(Self { f, grid })
    }

    fn near(&self, lat: f64, lon: f64) -> Vec<(usize, f64)> {
        let (la, lo) = (self.f.slice::<f64>("p_lat"), self.f.slice::<f64>("p_lon"));
        let (ci, cj) = ((lat / CELL).floor() as i32, (lon / CELL).floor() as i32);
        let n = (ACCESS_M / 400.0).ceil() as i32 + 1;
        let mut out = Vec::new();
        for i in ci - n..=ci + n {
            for j in cj - n..=cj + n {
                for &s in self.grid.get(&(i, j)).map_or(&[][..], Vec::as_slice) {
                    let m = dist_m(lat, lon, la[s as usize], lo[s as usize]);
                    if m <= ACCESS_M {
                        out.push((s as usize, m));
                    }
                }
            }
        }
        out
    }

    /// To board a route whose vehicle should have left its first stop at `tc` or later: the departure taken
    /// from the first stop and the wait. None outside its hours, or if the next one is over two hours away.
    fn board(&self, route: usize, day: usize, tc: f64) -> Option<(f64, i32)> {
        let (ff, fd) = (self.f.slice::<i32>("f_first"), self.f.slice::<i32>("f_datos"));
        let at = route * 7 + day;
        for band in fd[3 * ff[at] as usize..3 * ff[at + 1] as usize].chunks_exact(3) {
            if band[0] as f64 <= tc && tc <= band[1] as f64 {
                return Some((tc + (band[2] / 2) as f64, band[2] / 2));
            }
        }
        let (sf, sd) = (self.f.slice::<i32>("s_first"), self.f.slice::<i32>("s_datos"));
        let times = &sd[sf[at] as usize..sf[at + 1] as usize];
        let k = times.partition_point(|t| (*t as f64) < tc);
        let next = *times.get(k)? as f64;
        (next - tc < MAX_WAIT as f64).then(|| (next, (next - tc) as i32))
    }

    /// `day`: 0 is Monday; `now`: seconds since midnight. The journey that arrives first.
    pub fn route(&self, lat1: f64, lon1: f64, lat2: f64, lon2: f64, day: usize, now: f64) -> Option<Value> {
        let f = &self.f;
        let stops = f.slice::<f64>("p_lat").len();
        let (rf, rs, ro) = (f.slice::<i32>("r_first"), f.slice::<i32>("r_paradas"), f.slice::<i32>("r_offs"));
        let (pf, pr, pp) = (f.slice::<i32>("pasan_first"), f.slice::<i32>("pasan_r"), f.slice::<i32>("pasan_pos"));
        let (wf, wa, ws) = (f.slice::<i32>("pie_first"), f.slice::<i32>("pie_a"), f.slice::<i32>("pie_seg"));
        let mut best = vec![f64::INFINITY; stops];
        let mut rounds: Vec<HashMap<usize, (f64, From)>> = vec![HashMap::new()];
        for (s, m) in self.near(lat1, lon1) {
            let t = now + m / WALK;
            rounds[0].insert(s, (t, From::Start(m)));
            best[s] = t;
        }
        let goals = self.near(lat2, lon2);
        if rounds[0].is_empty() || goals.is_empty() {
            return None;
        }
        let mut marked: Vec<usize> = rounds[0].keys().copied().collect();
        for _ in 0..ROUNDS {
            let limit = goals.iter().map(|&(s, m)| best[s] + m / WALK).fold(f64::INFINITY, f64::min);
            let mut starts: HashMap<usize, usize> = HashMap::new();
            for &s in &marked {
                for k in pf[s] as usize..pf[s + 1] as usize {
                    let (r, pos) = (pr[k] as usize, pp[k] as usize);
                    let e = starts.entry(r).or_insert(pos);
                    *e = (*e).min(pos);
                }
            }
            let mut current: HashMap<usize, (f64, From)> = HashMap::new();
            {
                let previous = rounds.last().unwrap();
                for (&r, &pos0) in &starts {
                    let (first, last) = (rf[r] as usize, rf[r + 1] as usize);
                    let mut aboard: Option<(f64, usize, i32)> = None;
                    for pos in pos0..last - first {
                        let s = rs[first + pos] as usize;
                        let off = ro[first + pos] as f64;
                        if let Some((dep, on, wait)) = aboard {
                            let t = dep + off;
                            if t < best[s] && t < limit {
                                best[s] = t;
                                current.insert(s, (t, From::Ride { route: r, on, off: pos, wait }));
                            }
                        }
                        if let Some(&(t, _)) = previous.get(&s) {
                            if let Some((dep, wait)) = self.board(r, day, t - off) {
                                if aboard.is_none_or(|a| dep < a.0) {
                                    aboard = Some((dep, pos, wait));
                                }
                            }
                        }
                    }
                }
            }
            let rides: Vec<(usize, f64)> = current.iter().map(|(s, v)| (*s, v.0)).collect();
            for (s, t0) in rides {
                for k in wf[s] as usize..wf[s + 1] as usize {
                    let (j, secs) = (wa[k] as usize, ws[k]);
                    let t = t0 + secs as f64;
                    if t < best[j] {
                        best[j] = t;
                        current.insert(j, (t, From::Walk { stop: s, secs }));
                    }
                }
            }
            if current.is_empty() {
                break;
            }
            marked = current.keys().copied().collect();
            rounds.push(current);
        }
        let mut end: Option<(f64, usize, usize, f64)> = None;
        for (k, round) in rounds.iter().enumerate().skip(1) {
            for &(s, m) in &goals {
                if let Some(&(t, _)) = round.get(&s) {
                    let t = t + m / WALK;
                    if end.is_none_or(|e| t < e.0) {
                        end = Some((t, k, s, m));
                    }
                }
            }
        }
        Some(self.journey(&rounds, end?, (lat1, lon1), (lat2, lon2), now))
    }

    fn journey(&self, rounds: &[HashMap<usize, (f64, From)>], end: (f64, usize, usize, f64), a: (f64, f64), b: (f64, f64), now: f64) -> Value {
        let f = &self.f;
        let (la, lo) = (f.slice::<f64>("p_lat"), f.slice::<f64>("p_lon"));
        let names = f.texts("p_nombre");
        let (rf, rs, ro) = (f.slice::<i32>("r_first"), f.slice::<i32>("r_paradas"), f.slice::<i32>("r_offs"));
        let (lines, modes, colors, heads) = (f.texts("r_linea"), f.texts("r_modo"), f.texts("r_color"), f.texts("r_destino"));
        let point = |s: usize| [lo[s], la[s]];
        let (t_end, mut k, mut s, m_end) = end;
        let mut legs: Vec<Value> = vec![json!({"tipo": "pie", "metros": (m_end * 1.3).round() as i64, "segundos": (m_end / WALK).round() as i64,
            "coords": [point(s), [b.1, b.0]], "hasta": "destino"})];
        loop {
            match rounds[k][&s].1 {
                From::Walk { stop, secs } => {
                    legs.push(json!({"tipo": "pie", "metros": (secs as f64 * WALK * 1.3).round() as i64, "segundos": secs,
                        "coords": [point(stop), point(s)], "hasta": names.get(s)}));
                    s = stop;
                }
                From::Ride { route, on, off, wait } => {
                    let first = rf[route] as usize;
                    let path: Vec<[f64; 2]> = (on..=off).map(|p| point(rs[first + p] as usize)).collect();
                    let (from, to) = (rs[first + on] as usize, rs[first + off] as usize);
                    legs.push(json!({"tipo": modes.get(route), "linea": lines.get(route), "destino": heads.get(route), "color": colors.get(route),
                        "desde": names.get(from), "hasta": names.get(to), "paradas": off - on, "espera": wait,
                        "segundos": ro[first + off] - ro[first + on], "coords": path}));
                    s = from;
                    k -= 1;
                }
                From::Start(m) => {
                    legs.push(json!({"tipo": "pie", "metros": (m * 1.3).round() as i64, "segundos": (m / WALK).round() as i64,
                        "coords": [[a.1, a.0], point(s)], "hasta": names.get(s)}));
                    break;
                }
            }
        }
        legs.reverse();
        let mut merged: Vec<Value> = Vec::new(); // two walks in a row are one
        for leg in legs {
            match merged.last_mut() {
                Some(last) if last["tipo"] == "pie" && leg["tipo"] == "pie" => {
                    last["metros"] = json!(last["metros"].as_i64().unwrap() + leg["metros"].as_i64().unwrap());
                    last["segundos"] = json!(last["segundos"].as_i64().unwrap() + leg["segundos"].as_i64().unwrap());
                    let extra: Vec<Value> = leg["coords"].as_array().unwrap()[1..].to_vec();
                    last["coords"].as_array_mut().unwrap().extend(extra);
                    last["hasta"] = leg["hasta"].clone();
                }
                _ => merged.push(leg),
            }
        }
        let coords: Vec<Value> = merged.iter().flat_map(|l| l["coords"].as_array().unwrap().clone()).collect();
        json!({"segundos": (t_end - now).round() as i64, "llegada": t_end.round() as i64, "tramos": merged, "coords": coords})
    }
}
