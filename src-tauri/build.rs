fn main() {
    // EasyTier's SDK links the optional Windows WinPcap backend through pnet.
    // Delay-load it so startup does not fail before the mesh is used; the
    // x64 runtime DLL is bundled for the interface path reached after a peer
    // comes online.
    if cfg!(windows) {
        println!("cargo:rustc-link-arg=/DELAYLOAD:packet.dll");
        println!("cargo:rustc-link-arg=/DEFAULTLIB:delayimp.lib");
    }
    tauri_build::build();
}
