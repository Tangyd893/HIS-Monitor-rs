fn main() -> Result<(), Box<dyn std::error::Error>> {
    prost_build::compile_protos(
        &[
            "src/protocol/proto/monitor.proto",
            "src/protocol/proto/alert.proto",
        ],
        &["src/protocol/proto/"],
    )?;
    Ok(())
}
