//! `pnpm bindings`: regenerates `apps/ui/src/bindings.ts` without starting the app.

fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../apps/ui/src/bindings.ts");
    daimos_lib::bindings::export(&path).expect("export bindings");
    println!("wrote {}", path.display());
}
