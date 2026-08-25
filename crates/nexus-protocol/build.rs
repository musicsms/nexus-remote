fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=../../proto/nexus.proto");
    prost_build::compile_protos(&["../../proto/nexus.proto"], &["../../proto/"])
}
