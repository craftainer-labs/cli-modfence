// modfence: layer=public

use std::collections::{BTreeMap, HashMap as Map};
use std::fmt::{self, Display};
use crate::util::*;
use self::model::{Engine, deep::Thing as Other};
use ::serde::Serialize;
extern crate alloc as heap;

pub mod model;
mod util;

pub mod nested {
    use super::Config;
    use super::super::outside;
    mod leaf;

    pub fn helper() -> Config {
        Config::new()
    }
}

// modfence: layer=domain
/// The entry point.
#[derive(Debug)]
pub struct Config {
    retries: u32,
}

pub enum Mode {
    Fast,
    Slow,
}

pub trait Runner {
    fn run(&self) -> u32;

    fn twice(&self) -> u32 {
        self.run() + self.run()
    }
}

impl Config {
    // modfence: owner=core
    #[must_use]
    pub fn new() -> Self {
        Self::with_retries(3)
    }

    fn with_retries(retries: u32) -> Self {
        fn clamp(value: u32) -> u32 {
            value.min(10)
        }
        Config { retries: clamp(retries) }
    }
}

impl Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.retries)
    }
}

impl Config {
    pub fn describe(&self) -> String {
        let parsed = "7".parse::<u8>();
        Engine::start(self).stop(parsed)
    }
}

pub const LIMIT: usize = limit();
static NAME: &str = "modfence";
type Pair = (u8, u8);

#[pyfunction]
#[pyo3(name = "plus")]
pub fn add(a: u8, b: u8) -> u8 {
    a + b
}

#[napi(js_name = "subtract")]
#[derive(Debug)]
pub fn sub(a: u8, b: u8) -> u8 {
    a - b
}

#[wasm_bindgen]
pub fn mul(a: u8, b: u8) -> u8 {
    a * b
}

pub fn run() {
    let f = |x: u8| util::double(x);
    f(1);
    helper_call(); // modfence: ignore[layers] reason="entry"
}
