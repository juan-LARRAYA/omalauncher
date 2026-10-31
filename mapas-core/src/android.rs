//! The JNI side of `dev.omalauncher.MapasCore`: every call takes and returns JSON text.

use crate::Engine;
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jdouble, jint, jstring};
use jni::JNIEnv;
use serde_json::{json, Value};
use std::sync::Mutex;

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

fn answer(env: &mut JNIEnv, result: Result<Option<Value>, String>) -> jstring {
    let text = match result {
        Ok(Some(v)) => v.to_string(),
        Ok(None) => json!({"error": "sin resultado"}).to_string(),
        Err(e) => json!({"error": e}).to_string(),
    };
    env.new_string(text).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn with_engine<T>(f: impl FnOnce(&mut Engine) -> Result<T, String>) -> Result<T, String> {
    let mut guard = ENGINE.lock().map_err(|_| "motor bloqueado".to_string())?;
    f(guard.as_mut().ok_or("motor sin abrir")?)
}

#[no_mangle]
pub extern "system" fn Java_dev_omalauncher_MapasCore_open(mut env: JNIEnv, _: JClass, dir: JString) {
    if let Ok(dir) = env.get_string(&dir) {
        let dir: String = dir.into();
        if let Ok(mut guard) = ENGINE.lock() {
            *guard = Some(Engine::new(&dir));
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_omalauncher_MapasCore_search(mut env: JNIEnv, _: JClass, q: JString, lat: jdouble, lon: jdouble) -> jstring {
    let q: String = env.get_string(&q).map(Into::into).unwrap_or_default();
    let r = with_engine(|e| e.search(&q, Some((lat, lon)))).map(Some);
    answer(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_dev_omalauncher_MapasCore_car(mut env: JNIEnv, _: JClass, lat1: jdouble, lon1: jdouble, lat2: jdouble, lon2: jdouble) -> jstring {
    let r = with_engine(|e| e.car((lat1, lon1), (lat2, lon2)));
    answer(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_dev_omalauncher_MapasCore_transit(mut env: JNIEnv, _: JClass, lat1: jdouble, lon1: jdouble, lat2: jdouble, lon2: jdouble, day: jint, now: jdouble) -> jstring {
    let r = with_engine(|e| e.transit((lat1, lon1), (lat2, lon2), day as usize, now));
    answer(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_dev_omalauncher_MapasCore_guideStart(_: JNIEnv, _: JClass) -> jboolean {
    with_engine(|e| Ok(e.guide_start())).unwrap_or(false) as jboolean
}

#[no_mangle]
pub extern "system" fn Java_dev_omalauncher_MapasCore_guideUpdate(mut env: JNIEnv, _: JClass, lat: jdouble, lon: jdouble) -> jstring {
    let r = with_engine(|e| e.guide_update(lat, lon)).map(Some);
    answer(&mut env, r)
}
