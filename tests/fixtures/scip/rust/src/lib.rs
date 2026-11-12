pub mod helper {
    pub fn run() -> u32 {
        1
    }
}

pub struct Svc;

impl Svc {
    pub fn run_it(&self) -> u32 {
        2
    }
}

use helper::run as go;

pub fn entry() -> u32 {
    let svc = Svc;
    svc.run_it() + go()
}
