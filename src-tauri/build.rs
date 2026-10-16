fn main() {
    // D-Sep.exe loads the DLLs it imports from System32 only, never from its own folder: a portable copy kept in
    // Downloads or on a card could otherwise pick up a planted dwmapi.dll. 0x800 is LOAD_LIBRARY_SEARCH_SYSTEM32.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
    tauri_build::build()
}
