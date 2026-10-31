//! Runs the engine from a terminal, to compare it with the Python one: one JSON request per stdin line.
//! {"q": "...", "lat": .., "lon": ..} | {"a": [lat, lon], "b": [lat, lon]} | the same with "dia" and "ahora".

use mapas_core::Engine;
use serde_json::{json, Value};
use std::io::BufRead;
use std::time::Instant;

fn point(v: &Value) -> (f64, f64) {
    (v[0].as_f64().unwrap(), v[1].as_f64().unwrap())
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: mapas-cli DATA_DIR");
    let mut engine = Engine::new(&dir);
    for line in std::io::stdin().lock().lines() {
        let req: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let t = Instant::now();
        let res = if !req["p"].is_null() {
            let p = point(&req["p"]);
            if req["inicio"].as_bool() == Some(true) {
                engine.guide_start();
            }
            engine.guide_update(p.0, p.1).map(Some)
        } else if let Some(q) = req["q"].as_str() {
            engine.search(q, req["lat"].as_f64().zip(req["lon"].as_f64())).map(Some)
        } else if req["dia"].is_null() {
            engine.car(point(&req["a"]), point(&req["b"]))
        } else {
            engine.transit(point(&req["a"]), point(&req["b"]), req["dia"].as_u64().unwrap() as usize, req["ahora"].as_f64().unwrap())
        };
        println!("{}", json!({"ms": t.elapsed().as_secs_f64() * 1000.0, "res": res.unwrap()}));
    }
}
