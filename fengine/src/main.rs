fn main() {
    if let Err(error) = framelean_engine::engine_entry() {
        eprintln!("{error}");
        println!("{}", serde_json::json!({"type":"host_error","error":error}));
        std::process::exit(1);
    }
}
