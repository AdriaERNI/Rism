//! A/B parity harness: run the same fixture through Rism's logfmt and
//! compare against Prism's log.py output (written by the Python twin).
//! `cargo run --example logfmt_parity /tmp/parity_in.json > rism.json`

#![allow(clippy::expect_used)]

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: logfmt_parity <fixture.json>");
    let f = std::fs::File::open(path).expect("fixture");
    let d: serde_json::Value = serde_json::from_reader(f).expect("valid json");
    let tc = rism::logfmt::truncate_params(d.get("params").expect("params"));
    let tr = rism::logfmt::truncate_result(d.get("res").expect("res"));
    println!("{}", serde_json::json!({"tc": tc, "tr": tr}));
}
