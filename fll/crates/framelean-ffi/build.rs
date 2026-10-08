use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let target = env::var("TARGET").expect("Cargo sets TARGET");

    match target.as_str() {
        "aarch64-apple-darwin" | "x86_64-apple-darwin" => {
            let export_list = out_dir.join("framelean_fll.exports");
            fs::write(&export_list, "_framelean_fll_get_api\n").expect("write macOS export list");
            println!(
                "cargo:rustc-link-arg-cdylib=-Wl,-exported_symbols_list,{}",
                export_list.display()
            );
            println!("cargo:rustc-link-arg-cdylib=-Wl,-install_name,@rpath/libframelean_fll.dylib");
        }
        "aarch64-pc-windows-msvc" | "x86_64-pc-windows-msvc" => {
            let definition = out_dir.join("framelean_fll.def");
            fs::write(
                &definition,
                "LIBRARY framelean_fll\nEXPORTS\n    framelean_fll_get_api\n",
            )
            .expect("write Windows export definition");
            println!("cargo:rustc-link-arg-cdylib=/DEF:{}", definition.display());
        }
        _ if target.contains("windows-gnu") => {
            let export_list = out_dir.join("framelean_fll.exports");
            fs::write(
                &export_list,
                "{\n  global: framelean_fll_get_api;\n  local: *;\n};\n",
            )
            .expect("write GNU Windows export list");
            println!(
                "cargo:rustc-link-arg-cdylib=-Wl,--version-script={}",
                export_list.display()
            );
        }
        _ => {
            let export_list = out_dir.join("framelean_fll.exports");
            fs::write(
                &export_list,
                "{\n  global: framelean_fll_get_api;\n  local: *;\n};\n",
            )
            .expect("write ELF export version script");
            println!(
                "cargo:rustc-link-arg-cdylib=-Wl,--version-script={}",
                export_list.display()
            );
        }
    }
}
