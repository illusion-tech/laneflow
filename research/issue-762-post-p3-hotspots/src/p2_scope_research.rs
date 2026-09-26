//! #770 使用共享平衡采集器，保持 #763 的固定协议和历史证据。
#[allow(dead_code)]
mod cache_research;
use cache_research::io;

use std::{error::Error, path::Path};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "7bdf1f0ee4ae436ffc688903899ce9d16f89e41b";
const EXPERIMENT: cache_research::Experiment = cache_research::Experiment {
    baseline: BASE,
    protocol: "p2-scope-abba-v1",
    count_p2: true,
};
fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}
fn run() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("prepare") if a.len()==5=>cache_research::export_for(Path::new(&a[4]),&a[1],&a[2],&a[3],EXPERIMENT),
        Some("run") if a.len()==5=>cache_research::capture_for(&a[1],Path::new(&a[2]),Path::new(&a[3]),Path::new(&a[4]),EXPERIMENT),
        Some("analyze"|"verify") if a.len()==3=> { let raw=Path::new(&a[1]); let out=Path::new(&a[2]); let v=cache_research::analyze_for(raw,EXPERIMENT)?; if a[0]=="verify" {need(v==io::read_json(out)?,"published mismatch")?; println!("verified"); Ok(())} else {io::outside(raw,out)?;io::write_new(out,&v)} },
        _=>Err("prepare <base|candidate> <plain|detail> <commit> <root> | run <plain|detail> <root> <inputs> <new-raw> | analyze|verify <raw> <results>".into())
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
