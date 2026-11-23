// Compiles the wire contract with protox, a pure-Rust protobuf compiler, so
// building needs no `protoc`.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto");
    let descriptors = protox::compile(["exilence.proto"], ["proto"])?;
    tonic_prost_build::configure().compile_fds(descriptors)?;
    Ok(())
}
