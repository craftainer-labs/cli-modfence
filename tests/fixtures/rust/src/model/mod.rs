//! The engine.
use super::Config;
use crate::util::{self as helpers, double};

pub mod deep;

pub struct Engine;

impl Engine {
    pub fn start(config: &Config) -> Engine {
        helpers::double(1);
        Engine
    }

    pub fn stop(&self, parsed: Result<u8, std::num::ParseIntError>) {}
}
