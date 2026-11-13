#[pyfunction(text = r#"a, name = "no""#, name = "raw")]
fn f1() {}

#[pyfunction(a = br"1,2", b = b"3,4", name = "bytes")]
fn f2() {}

#[pyfunction(q = '"', name = "quote")]
fn f3() {}

#[pyfunction(a = '\'', b = '\u{2c}', c = 'é', d = &'static str, e = '\\', name = "chars")]
fn f4() {}

#[pyfunction(text = "a\"b, name = \"no\"", name = "escaped")]
fn f5() {}

#[pyfunction(more = r##"x"# , name = "no""##, name = "hashes")]
fn f6() {}

#[pyfunction(open = '(', name = "paren")]
fn f7() {}
