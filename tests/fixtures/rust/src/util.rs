pub fn double(x: u8) -> u8 {
    triple(x) - x
}

fn triple(x: u8) -> u8 {
    x * 3
}
