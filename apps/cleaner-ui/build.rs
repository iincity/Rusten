fn main() {
    println!("cargo:rerun-if-changed=resources/rusten-icon.rc");
    println!("cargo:rerun-if-changed=resources/generated/rusten.ico");
    embed_resource::compile("resources/rusten-icon.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("embed Rusten application icon");
}
