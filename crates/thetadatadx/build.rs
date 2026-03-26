fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(
            &["proto/endpoints.proto", "proto/v3_endpoints.proto"],
            &["proto"],
        )?;
    Ok(())
}
