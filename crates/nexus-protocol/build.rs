fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("PROTOC").is_err() {
        let protoc_path = protoc_bin_vendored::protoc_bin_path()?;
        std::env::set_var("PROTOC", protoc_path);
    }

    println!("cargo:rerun-if-changed=../../proto/nexus.proto");
    prost_build::compile_protos(&["../../proto/nexus.proto"], &["../../proto/"])?;
    Ok(())
}
