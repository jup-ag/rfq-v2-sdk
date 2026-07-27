fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use vendored protoc to avoid building C++ protobuf via autotools
    unsafe {
        std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);
    }

    tonic_prost_build::compile_protos("../protos/market_maker.proto")?;

    Ok(())
}
