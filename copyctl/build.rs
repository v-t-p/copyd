use std::io::Result;

fn main() -> Result<()> {
    prost_build::compile_protos(
        &["../copyd-protocol/proto/copyd.proto"],
        &["../copyd-protocol/proto"],
    )?;
    Ok(())
} 