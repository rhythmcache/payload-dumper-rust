fn main() -> Result<(), Box<dyn std::error::Error>> {
      let mut config = prost_build::Config::new();
    config.type_attribute(".", "#[derive(serde::Serialize)]");
    prost_build::compile_protos(&["proto/update_metadata.proto"], &["proto/"])?;
    Ok(())
}
