fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Automatically locate/build protoc using protobuf-src if PROTOC environment variable is not set
    if std::env::var("PROTOC").is_err() {
        std::env::set_var("PROTOC", protobuf_src::protoc());
    }

    println!("cargo:rerun-if-changed=../../proto/nexus.proto");
    prost_build::compile_protos(&["../../proto/nexus.proto"], &["../../proto/"])?;
    Ok(())
}
