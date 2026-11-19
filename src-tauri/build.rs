fn main() {
    // Date de compilation et version de l'extension livrée, affichées dans les réglages
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    println!("cargo:rustc-env=CAP_BUILD_MS={now}");
    let ext = std::fs::read_to_string("../extension/manifest.json").unwrap_or_default();
    let ver = ext
        .split("\"version\"")
        .nth(1)
        .and_then(|r| r.split('"').nth(1))
        .unwrap_or("?")
        .to_string();
    println!("cargo:rustc-env=CAP_EXT_VERSION={ver}");
    println!("cargo:rerun-if-changed=../extension/manifest.json");
    println!("cargo:rerun-if-changed=../ui");
    println!("cargo:rerun-if-changed=../locales");
    println!("cargo:rerun-if-changed=src");
    tauri_build::build()
}
