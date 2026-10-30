//! Car routes: A* over the graph of crossings. No turn restrictions or traffic yet.

use crate::bin_file::BinFile;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

const CELL: f64 = 0.005; // degrees, ~500 m: finds the nearest node
const MAX_KMH: f64 = 110.0; // bound that keeps the heuristic admissible

pub(crate) fn dist_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let x = (lon2 - lon1).to_radians() * ((lat1 + lat2) / 2.0).to_radians().cos();
    let y = (lat2 - lat1).to_radians();
    6_371_000.0 * x.hypot(y)
}

struct Item {
    priority: f64,
    cost: f64,
    node: u32,
}
impl PartialEq for Item {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Item {
    // BinaryHeap is a max-heap: the smallest priority first.
    fn cmp(&self, other: &Self) -> Ordering {
        other.priority.total_cmp(&self.priority)
    }
}

pub struct Router {
    f: BinFile,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

impl Router {
    pub fn open(path: &str) -> Result<Self, String> {
        let f = BinFile::open(path)?;
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        {
            let (lat, lon) = (f.slice::<f64>("lat"), f.slice::<f64>("lon"));
            let first = f.slice::<i32>("first");
            for n in 0..lat.len() {
                if first[n + 1] > first[n] {
                    grid.entry(((lat[n] / CELL).floor() as i32, (lon[n] / CELL).floor() as i32)).or_default().push(n as u32);
                }
            }
        }
        Ok(Self { f, grid })
    }

    fn nearest(&self, lat: f64, lon: f64) -> Option<(u32, f64)> {
        let (la, lo) = (self.f.slice::<f64>("lat"), self.f.slice::<f64>("lon"));
        let (ci, cj) = ((lat / CELL).floor() as i32, (lon / CELL).floor() as i32);
        let mut best: Option<(u32, f64)> = None;
        for r in 0..6i32 {
            for i in ci - r..=ci + r {
                for j in cj - r..=cj + r {
                    if (i - ci).abs().max((j - cj).abs()) != r {
                        continue;
                    }
                    for &n in self.grid.get(&(i, j)).map_or(&[][..], Vec::as_slice) {
                        let d = dist_m(lat, lon, la[n as usize], lo[n as usize]);
                        if best.is_none_or(|(_, b)| d < b) {
                            best = Some((n, d));
                        }
                    }
                }
            }
            if best.is_some_and(|(_, b)| b < r as f64 * CELL * 111_000.0) {
                break;
            }
        }
        best
    }

    pub fn route(&self, lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> Option<Value> {
        let (src, d_src) = self.nearest(lat1, lon1)?;
        let (dst, d_dst) = self.nearest(lat2, lon2)?;
        let f = &self.f;
        let (lat, lon) = (f.slice::<f64>("lat"), f.slice::<f64>("lon"));
        let (first, target) = (f.slice::<i32>("first"), f.slice::<i32>("target"));
        let secs = f.slice::<f32>("secs");
        let (tlat, tlon) = (lat[dst as usize], lon[dst as usize]);
        let speed = MAX_KMH / 3.6;
        let mut best = vec![f64::INFINITY; lat.len()];
        let mut prev = vec![-1i32; lat.len()]; // the edge each node was reached by
        let mut from = vec![0u32; lat.len()];
        best[src as usize] = 0.0;
        let mut heap = BinaryHeap::new();
        heap.push(Item { priority: dist_m(lat[src as usize], lon[src as usize], tlat, tlon) / speed, cost: 0.0, node: src });
        while let Some(Item { cost, node, .. }) = heap.pop() {
            if node == dst {
                break;
            }
            let n = node as usize;
            if cost > best[n] {
                continue;
            }
            for k in first[n] as usize..first[n + 1] as usize {
                let m = target[k] as usize;
                let c = cost + secs[k] as f64;
                if c < best[m] {
                    best[m] = c;
                    prev[m] = k as i32;
                    from[m] = node;
                    heap.push(Item { priority: c + dist_m(lat[m], lon[m], tlat, tlon) / speed, cost: c, node: m as u32 });
                }
            }
        }
        if !best[dst as usize].is_finite() {
            return None;
        }
        let mut edges = Vec::new();
        let mut n = dst;
        while n != src {
            edges.push(prev[n as usize] as usize);
            n = from[n as usize];
        }
        edges.reverse();
        Some(self.describe(&edges, best[dst as usize], d_src, d_dst, (lat1, lon1), (lat2, lon2)))
    }

    fn describe(&self, edges: &[usize], total: f64, d_src: f64, d_dst: f64, a: (f64, f64), b: (f64, f64)) -> Value {
        let f = &self.f;
        let (edge, meters) = (f.slice::<i32>("edge"), f.slice::<f32>("meters"));
        let (geom_first, coords) = (f.slice::<i32>("geom_first"), f.slice::<f64>("coords"));
        let names = f.slice::<i32>("names");
        let streets = f.texts("calle");
        let mut line = vec![[a.1, a.0]];
        let mut steps: Vec<(String, f64)> = Vec::new();
        let mut distance = 0.0;
        for &k in edges {
            let e = edge[k] as usize;
            let c = &coords[geom_first[e] as usize..geom_first[e + 1] as usize];
            line.extend(c.chunks_exact(2).map(|p| [p[1], p[0]]));
            let name = match streets.get(names[e] as usize) {
                "" => "calle sin nombre",
                s => s,
            };
            let m = meters[k] as f64;
            distance += m;
            match steps.last_mut() {
                Some(last) if last.0 == name => last.1 += m,
                _ => steps.push((name.to_string(), m)),
            }
        }
        line.push([b.1, b.0]);
        let pasos: Vec<Value> = steps.iter().map(|(c, m)| json!({"calle": c, "metros": m.round() as i64})).collect();
        json!({"coords": line, "segundos": total.round() as i64, "metros": distance.round() as i64,
               "acercamiento_m": (d_src + d_dst).round() as i64, "pasos": pasos})
    }
}
