//! Turn-by-turn guidance over a car route: where you are along it, the next turn, what's left, and whether you
//! left the route. Position only ever moves forward, so a street running the other way nearby can't pull it back.

use crate::ruteo::dist_m;
use serde_json::{json, Value};

const OFF_ROUTE_M: f64 = 50.0;
const OFF_ROUTE_FIXES: u32 = 3; // one bad GPS fix isn't leaving the route
const LOOK_AHEAD_M: f64 = 300.0;
const ARRIVED_M: f64 = 30.0;

struct Step {
    at: f64, // metres along the route where it starts
    street: String,
    turn: String,
}

pub struct Guide {
    line: Vec<[f64; 2]>, // lon, lat
    along: Vec<f64>,     // metres from the start to each point
    steps: Vec<Step>,
    seconds: f64,
    position: f64,
    off: u32,
}

/// The sentence for a step, as it's said here.
pub fn sentence(turn: &str, street: &str) -> String {
    match turn {
        "izquierda" => format!("Doblá a la izquierda en {street}"),
        "derecha" => format!("Doblá a la derecha en {street}"),
        "leve-izquierda" => format!("Tomá a la izquierda por {street}"),
        "leve-derecha" => format!("Tomá a la derecha por {street}"),
        "retoma" => format!("Pegá la vuelta por {street}"),
        "salida" => format!("Salí por {street}"),
        _ => format!("Seguí por {street}"),
    }
}

impl Guide {
    /// From the JSON of a car route: its `coords`, `segundos` and `pasos`.
    pub fn new(route: &Value) -> Option<Self> {
        let line: Vec<[f64; 2]> = route["coords"].as_array()?.iter().map(|c| Some([c[0].as_f64()?, c[1].as_f64()?])).collect::<Option<_>>()?;
        if line.len() < 2 {
            return None;
        }
        let mut along = vec![0.0];
        for w in line.windows(2) {
            along.push(along.last()? + dist_m(w[0][1], w[0][0], w[1][1], w[1][0]));
        }
        let mut steps = Vec::new();
        for p in route["pasos"].as_array()? {
            steps.push(Step { at: *along.get(p["indice"].as_u64()? as usize)?, street: p["calle"].as_str()?.to_string(), turn: p["giro"].as_str()?.to_string() });
        }
        Some(Self { line, along, steps, seconds: route["segundos"].as_f64()?, position: 0.0, off: 0 })
    }

    /// The closest point of the route ahead of `position`: metres along it, and metres away from it.
    fn snap(&self, lat: f64, lon: f64) -> (f64, f64) {
        let first = self.along.partition_point(|m| *m <= self.position).saturating_sub(1);
        let scale = lat.to_radians().cos();
        let mut best = (self.position, f64::INFINITY);
        for i in first..self.line.len() - 1 {
            if self.along[i] > self.position + LOOK_AHEAD_M {
                break;
            }
            let (a, b) = (self.line[i], self.line[i + 1]);
            let (dx, dy) = ((b[0] - a[0]) * scale, b[1] - a[1]);
            let (px, py) = ((lon - a[0]) * scale, lat - a[1]);
            let len = dx * dx + dy * dy;
            let t = if len > 0.0 { ((px * dx + py * dy) / len).clamp(0.0, 1.0) } else { 0.0 };
            let m = dist_m(lat, lon, a[1] + t * (b[1] - a[1]), a[0] + t * (b[0] - a[0]));
            if m < best.1 {
                best = ((self.along[i] + t * (self.along[i + 1] - self.along[i])).max(self.position), m);
            }
        }
        best
    }

    /// A new GPS fix: what the screen shows now.
    pub fn update(&mut self, lat: f64, lon: f64) -> Value {
        let (at, away) = self.snap(lat, lon);
        if away > OFF_ROUTE_M {
            self.off += 1;
        } else {
            self.off = 0;
            self.position = at;
        }
        let total = *self.along.last().unwrap();
        let left = total - self.position;
        let next = self.steps.iter().find(|s| s.at > self.position + 1.0);
        let (text, turn, to_turn) = match next {
            Some(s) => (sentence(&s.turn, &s.street), s.turn.as_str(), s.at - self.position),
            None => ("Seguí hasta el destino".to_string(), "destino", left),
        };
        json!({"texto": text, "giro": turn, "metros": to_turn.round() as i64, "restante_m": left.round() as i64,
               "restante_s": (self.seconds * left / total.max(1.0)).round() as i64,
               "fuera": self.off >= OFF_ROUTE_FIXES, "llego": left < ARRIVED_M})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 km east along a street, then 1 km north along another.
    fn route() -> Value {
        json!({"coords": [[-58.40, -34.60], [-58.39, -34.60], [-58.38909, -34.60], [-58.38909, -34.59]], "segundos": 200,
               "pasos": [{"calle": "Este", "giro": "salida", "indice": 0}, {"calle": "Norte", "giro": "izquierda", "indice": 2}]})
    }

    #[test]
    fn announces_the_next_turn_and_what_is_left() {
        let mut g = Guide::new(&route()).unwrap();
        let v = g.update(-34.60, -58.395);
        assert_eq!(v["texto"], "Doblá a la izquierda en Norte");
        assert_eq!(v["giro"], "izquierda");
        assert!((v["metros"].as_i64().unwrap() - 541).abs() < 15, "{v}");
        assert_eq!(v["fuera"], false);
    }

    #[test]
    fn after_the_last_turn_heads_to_the_destination_and_arrives() {
        let mut g = Guide::new(&route()).unwrap();
        g.update(-34.60, -58.395);
        g.update(-34.60, -58.3895);
        g.update(-34.598, -58.38909);
        let v = g.update(-34.595, -58.38909);
        assert_eq!(v["giro"], "destino");
        assert_eq!(v["llego"], false);
        g.update(-34.5915, -58.38909);
        assert_eq!(g.update(-34.5901, -58.38909)["llego"], true);
    }

    #[test]
    fn one_bad_fix_is_noise_three_are_leaving_the_route() {
        let mut g = Guide::new(&route()).unwrap();
        g.update(-34.60, -58.395);
        let far = -34.605;
        assert_eq!(g.update(far, -58.395)["fuera"], false);
        assert_eq!(g.update(-34.60, -58.3945)["fuera"], false);
        g.update(far, -58.395);
        g.update(far, -58.395);
        assert_eq!(g.update(far, -58.395)["fuera"], true);
    }

    #[test]
    fn never_goes_backwards() {
        let mut g = Guide::new(&route()).unwrap();
        let ahead = g.update(-34.60, -58.392)["restante_m"].as_i64().unwrap();
        let back = g.update(-34.60, -58.398)["restante_m"].as_i64().unwrap();
        assert_eq!(ahead, back);
    }

    #[test]
    fn says_it_as_it_is_said_here() {
        assert_eq!(sentence("derecha", "Av. Santa Fe"), "Doblá a la derecha en Av. Santa Fe");
        assert_eq!(sentence("recto", "Lima"), "Seguí por Lima");
    }
}
