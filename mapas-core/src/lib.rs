//! Mapas' engine: offline search, car routes and public transport. No network code at all.

mod bin_file;
pub mod busqueda;
pub mod ruteo;
pub mod transporte;

#[cfg(target_os = "android")]
mod android;

use serde_json::Value;

/// Everything the app asks, over the files `mapas-datos` builds, loaded on first use.
pub struct Engine {
    dir: String,
    search: Option<busqueda::Search>,
    router: Option<ruteo::Router>,
    network: Option<transporte::Network>,
}

impl Engine {
    pub fn new(dir: &str) -> Self {
        Self { dir: dir.to_string(), search: None, router: None, network: None }
    }

    pub fn search(&mut self, q: &str, near: Option<(f64, f64)>) -> Result<Value, String> {
        if self.search.is_none() {
            self.search = Some(busqueda::Search::open(&format!("{}/busqueda.sqlite", self.dir))?);
        }
        self.search.as_ref().unwrap().search(q, near, 8).map(Value::from).map_err(|e| e.to_string())
    }

    pub fn car(&mut self, a: (f64, f64), b: (f64, f64)) -> Result<Option<Value>, String> {
        if self.router.is_none() {
            self.router = Some(ruteo::Router::open(&format!("{}/grafo-auto.bin", self.dir))?);
        }
        Ok(self.router.as_ref().unwrap().route(a.0, a.1, b.0, b.1))
    }

    pub fn transit(&mut self, a: (f64, f64), b: (f64, f64), day: usize, now: f64) -> Result<Option<Value>, String> {
        if self.network.is_none() {
            self.network = Some(transporte::Network::open(&format!("{}/transporte.bin", self.dir))?);
        }
        Ok(self.network.as_ref().unwrap().route(a.0, a.1, b.0, b.1, day, now))
    }
}
