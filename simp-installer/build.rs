fn main() {
    println!("cargo:rerun-if-changed=installer.rc");
    println!("cargo:rerun-if-changed=assets/simp-installer.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("installer.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed installer icon");
    }
}
