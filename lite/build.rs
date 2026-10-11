fn main() {
    println!("cargo:rerun-if-changed=lolping-lite.rc");
    println!("cargo:rerun-if-changed=../build/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // The exe's icon in Explorer and Task Manager.
        embed_resource::compile("lolping-lite.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
