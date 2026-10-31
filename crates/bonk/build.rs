//! Icône et informations de version de `bonk.exe`. Seule la chaîne MSVC est servie : c'est celle des binaires
//! publiés, et le compilateur de ressources n'existe pas sur les autres hôtes.

fn main() {
    println!("cargo:rerun-if-changed=assets/bonk.ico");
    println!("cargo:rerun-if-changed=build.rs");
    let var = |name| std::env::var(name).unwrap_or_default();
    if var("CARGO_CFG_TARGET_OS") != "windows" || var("CARGO_CFG_TARGET_ENV") != "msvc" {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/bonk.ico")
        .set("ProductName", "Bonk")
        .set("FileDescription", "Bonk - bootable USB writer")
        .set("LegalCopyright", "FantasmaGlad, MIT");
    if let Err(error) = resource.compile() {
        println!("cargo:warning=ressources Windows : {error}");
        std::process::exit(1);
    }
}
