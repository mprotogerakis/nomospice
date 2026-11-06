fn main() {
    // libngspice is loaded at runtime (see ngspice.rs), not linked
    tauri_build::build()
}
