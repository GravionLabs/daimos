//! `pnpm bindings` regenerates `apps/ui/src/bindings.ts` without starting the app;
//! `--check` (part of `pnpm test`) fails when the checked-in file is stale.

fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../apps/ui/src/bindings.ts");
    if std::env::args().any(|a| a == "--check") {
        match daimos_lib::bindings::is_current(&path) {
            Ok(true) => println!("bindings.ts is current"),
            Ok(false) => {
                eprintln!("bindings.ts is stale: run `pnpm bindings`");
                std::process::exit(1);
            }
            Err(error) => {
                eprintln!("cannot check bindings.ts: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    daimos_lib::bindings::export(&path).expect("export bindings");
    println!("wrote {}", path.display());
}
